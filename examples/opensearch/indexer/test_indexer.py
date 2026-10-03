import asyncio
import inspect
import json
import random
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, Mock, patch
from uuid import UUID

from kafka import TopicPartition
from opensearchpy.exceptions import ConflictError, RequestError

import indexer


TOPIC = "search_demo.products"
CONFIG = {"index": TOPIC, "fields": {"name": {"tags": ["search"]}}}
STREAM = {"topic": TOPIC, "topic_id": "12345678-1234-1234-1234-123456789abc", "partition": 0}


def record(offset=0, value=b'{"_id":"p1","name":"first"}', key=b'{"_id":"p1"}'):
    return SimpleNamespace(topic=TOPIC, partition=0, offset=offset, value=value, key=key)


class StreamTests(unittest.TestCase):
    def metadata(self, **changes):
        topic = {"topic": TOPIC, "topic_id": UUID(STREAM["topic_id"]),
                 "error_code": 0, "partitions": [{"partition": 0, "error_code": 0}]}
        topic.update(changes)
        return Mock(describe_topics=Mock(return_value=[topic]))

    def test_uuid_is_normalized(self):
        self.assertEqual(indexer.describe_stream(self.metadata(), TOPIC), STREAM)

    def test_flexible_metadata_protocol_field_names(self):
        admin = Mock(describe_topics=Mock(return_value=[{
            "name": TOPIC, "topic_id": STREAM["topic_id"], "error_code": 0,
            "partitions": [{"partition_index": 0, "error_code": 0}]
        }]))
        self.assertEqual(indexer.describe_stream(admin, TOPIC), STREAM)

    def test_missing_or_zero_uuid_is_rejected(self):
        for value in (None, UUID(int=0)):
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                indexer.describe_stream(self.metadata(topic_id=value), TOPIC)

    def test_multiple_partitions_are_rejected(self):
        with self.assertRaises(RuntimeError):
            indexer.describe_stream(self.metadata(partitions=[{"partition": 0}, {"partition": 1}]), TOPIC)

    def test_broker_error_is_rejected(self):
        with self.assertRaises(RuntimeError):
            indexer.describe_stream(self.metadata(error_code=3), TOPIC)

    def test_mapping_has_durable_contract(self):
        body, _, _ = indexer.build_index_body(CONFIG, STREAM)
        self.assertEqual(body["mappings"]["_meta"]["stream"], STREAM)
        self.assertEqual(body["mappings"]["properties"]["_sync_deleted"], {"type": "boolean"})
        self.assertEqual(body["mappings"]["properties"]["_mongo_id"], {"type": "keyword"})

    def test_different_configuration_changes_contract(self):
        a = indexer.build_index_body(CONFIG, STREAM)[0]
        b = indexer.build_index_body({**CONFIG, "fields": {}}, STREAM)[0]
        self.assertNotEqual(a["mappings"]["_meta"], b["mappings"]["_meta"])

    def test_existing_index_with_changed_uuid_fails(self):
        client = Mock()
        client.indices.exists.return_value = True
        body = indexer.build_index_body(CONFIG, {**STREAM, "topic_id": "other"})[0]
        client.indices.get_mapping.return_value = {TOPIC: body}
        with self.assertRaises(RuntimeError):
            indexer.ensure_index(client, TOPIC, CONFIG, STREAM)

    def test_concurrent_create_validates_winner(self):
        client = Mock()
        client.indices.exists.return_value = False
        client.indices.create.side_effect = RequestError(400, "resource_already_exists_exception")
        body = indexer.build_index_body(CONFIG, STREAM)[0]
        client.indices.get_mapping.return_value = {TOPIC: body}
        indexer.ensure_index(client, TOPIC, CONFIG, STREAM)

    def test_other_create_errors_are_not_swallowed(self):
        client = Mock()
        client.indices.exists.return_value = False
        client.indices.create.side_effect = RequestError(400, "invalid_index_name_exception")
        with self.assertRaises(RequestError):
            indexer.ensure_index(client, TOPIC, CONFIG, STREAM)

    def test_reserved_fields_rejected(self):
        with self.assertRaises(ValueError):
            indexer.build_index_body({**CONFIG, "fields": {"_sync_deleted": {}}}, STREAM)


class WriteTests(unittest.TestCase):
    def setUp(self):
        self.client = Mock()

    def write(self, message=None):
        return indexer.write_record(self.client, CONFIG, STREAM, message or record())

    def test_type_distinct_ids_get_distinct_opensearch_keys(self):
        oid = {"$oid": "507f1f77bcf86cd799439011"}
        hex_id = "507f1f77bcf86cd799439011"
        cases = [
            ({"_id": 1, "name": "n"}, {"_id": 1}, "1"),
            ({"_id": "1", "name": "n"}, {"_id": "1"}, '"1"'),
            ({"_id": oid}, {"_id": oid}, '{"$oid":"507f1f77bcf86cd799439011"}'),
            ({"_id": hex_id}, {"_id": hex_id}, f'"{hex_id}"'),
        ]
        keys = []
        for value, key, expected in cases:
            self.client.reset_mock()
            self.write(record(
                value=json.dumps(value).encode(),
                key=json.dumps(key).encode(),
            ))
            opensearch_id = self.client.index.call_args.kwargs["id"]
            self.assertEqual(opensearch_id, expected)
            self.assertEqual(self.client.index.call_args.kwargs["body"]["_mongo_id"], expected)
            keys.append(opensearch_id)
        self.assertEqual(len(set(keys)), len(cases))

    def test_full_replace_uses_external_offset_version(self):
        self.write(record(offset=8))
        args = self.client.index.call_args.kwargs
        self.assertEqual(args["version"], 9)
        self.assertEqual(args["version_type"], "external")
        self.assertEqual(args["id"], '"p1"')
        self.assertNotIn("_id", args["body"])
        self.assertEqual(args["body"]["_mongo_id"], '"p1"')
        self.assertFalse(args["body"]["_sync_deleted"])
        self.assertEqual(args["body"]["_sync_topic_id"], STREAM["topic_id"])

    def test_delete_is_persistent_tombstone_without_embedding(self):
        self.write(record(offset=9, value=None))
        args = self.client.index.call_args.kwargs
        self.assertEqual(args["id"], '"p1"')
        self.assertTrue(args["body"]["_sync_deleted"])
        self.assertEqual(args["pipeline"], "_none")
        self.assertNotIn("name", args["body"])
        self.client.delete.assert_not_called()

    def test_duplicate_or_stale_same_stream_is_success(self):
        for version in (1, 10):
            with self.subTest(version=version):
                self.client.index.side_effect = ConflictError(409, "version_conflict_engine_exception")
                self.client.get.return_value = {"_version": version, "_source": {
                    "_sync_topic_id": STREAM["topic_id"], "_sync_deleted": True}}
                self.write()

    def test_foreign_conflict_fails_closed(self):
        self.client.index.side_effect = ConflictError(409, "version_conflict_engine_exception")
        for source in ({}, {"_sync_topic_id": "other"}):
            with self.subTest(source=source), self.assertRaises(RuntimeError):
                self.client.get.return_value = {"_version": 10, "_source": source}
                self.write()

    def test_invalid_records_never_write(self):
        for message in (record(value=b"broken"), record(value=b"[]"),
                        record(value=b'{}'), record(value=None, key=None),
                        record(value=b'{"_id":null}'), record(key=b'{"_id":"other"}')):
            with self.subTest(message=message), self.assertRaises((ValueError, RuntimeError)):
                self.write(message)
        self.client.index.assert_not_called()

    def test_wrong_partition_never_writes(self):
        message = record()
        message.partition = 1
        with self.assertRaises(RuntimeError):
            self.write(message)

    def test_write_error_propagates(self):
        self.client.index.side_effect = TimeoutError()
        with self.assertRaises(TimeoutError):
            self.write()

    def test_replays_in_random_order_converge_without_resurrection(self):
        class VersionedStore:
            def __init__(self):
                self.documents = {}

            def index(self, **request):
                previous = self.documents.get(request["id"], {"_version": 0})
                if request["version"] <= previous["_version"]:
                    raise ConflictError(409, "version_conflict_engine_exception")
                self.documents[request["id"]] = {
                    "_version": request["version"], "_source": request["body"]}

            def get(self, *, index, id):
                return self.documents[id]

        with patch.object(indexer, "log"):
            for seed in range(64):
                store = VersionedStore()
                messages = [record(offset=i, value=None if i % 7 == 0 else json.dumps({
                    "_id": "p1", "name": str(i)}).encode()) for i in range(50)]
                messages[-1] = record(offset=49, value=None)
                replay = messages * 2
                random.Random(seed).shuffle(replay)
                for message in replay:
                    indexer.write_record(store, CONFIG, STREAM, message)
                self.assertEqual(store.get(index=TOPIC, id='"p1"')["_version"], 50)
                self.assertTrue(store.get(index=TOPIC, id='"p1"')["_source"]["_sync_deleted"])
                indexer.write_record(store, CONFIG, STREAM, record(offset=50))
                for message in messages:
                    indexer.write_record(store, CONFIG, STREAM, message)
                self.assertFalse(store.get(index=TOPIC, id='"p1"')["_source"]["_sync_deleted"])


class ProjectionTests(unittest.TestCase):
    def test_role_file_updates_are_thread_safe(self):
        with ThreadPoolExecutor(max_workers=8) as pool:
            list(pool.map(indexer.set_role, ["active", "standby", "stopped"] * 100))
        self.assertIn(Path("/tmp/indexer-role").read_text(), {"active", "standby", "stopped"})

    def test_nested_projection_omits_unconfigured_fields(self):
        config = {"fields": {"details.name": {"sourcePath": "original.title"}}}
        source = {"_id": "id", "original": {"title": "test"}, "secret": "hidden"}
        self.assertEqual(indexer.project_document(TOPIC, config, source), {
            "_id": "id", "_mongo_id": '"id"', "_mongo_namespace": TOPIC,
            "details": {"name": "test"}})

    def test_integer_id_keeps_json_for_typed_lookup(self):
        projected = indexer.project_document(TOPIC, {"fields": {}}, {"_id": 1, "name": "n"})
        self.assertEqual(projected["_id"], 1)
        self.assertEqual(projected["_mongo_id"], "1")

    def test_extended_object_id_keeps_original_json(self):
        oid = {"$oid": "507f1f77bcf86cd799439011"}
        projected = indexer.project_document(TOPIC, {"fields": {}}, {"_id": oid})
        self.assertEqual(projected["_id"], "507f1f77bcf86cd799439011")
        self.assertEqual(projected["_mongo_id"], '{"$oid":"507f1f77bcf86cd799439011"}')

    def test_mongo_extended_ids(self):
        for key in ("$oid", "$uuid"):
            encoded = json.dumps({"documentKey": {"_id": {key: "document-id"}}}).encode()
            self.assertEqual(indexer.document_id_from_key(encoded), "document-id")

    def test_missing_key_shapes(self):
        for encoded in (None, b"", b"[]", b"null", b'{"documentKey":[]}'):
            self.assertIsNone(indexer.document_id_from_key(encoded))

    def test_env_expansion_is_recursive(self):
        with patch.dict(indexer.os.environ, {"TEST_MODEL": "model"}):
            self.assertEqual(indexer.expand_env({"a": ["${TEST_MODEL}", "${UNSET:-fallback}"]}),
                             {"a": ["model", "fallback"]})

    def test_scalar_mapping_validation(self):
        for kind in ("keyword", "integer", "long", "float", "double", "boolean", "date"):
            self.assertEqual(indexer.mapping_type({"type": kind}), {"type": kind})
        with self.assertRaises(ValueError):
            indexer.mapping_type({"type": "unsupported"})

    def test_vector_mapping_and_default_pipelines(self):
        config = {"index": TOPIC, "vector": {"modelId": "test-model", "dimension": 12},
                  "fields": {"description": {"tags": ["vectorSearch"]}}}
        body, fields, model = indexer.build_index_body(config, STREAM)
        self.assertEqual(fields, [("description", "description_embedding")])
        self.assertEqual(model, "test-model")
        self.assertEqual(body["mappings"]["properties"]["description_embedding"]["dimension"], 12)
        self.assertIn("default_pipeline", body["settings"]["index"])
        self.assertIn("default_pipeline", body["settings"]["index"]["search"])


class CommitTests(unittest.TestCase):
    def setUp(self):
        self.consumer = Mock()
        self.tp = TopicPartition(TOPIC, 0)
        self.consumer.assignment.return_value = {self.tp}

    def test_commit_only_after_write(self):
        events = []
        self.consumer.commit.side_effect = lambda **kwargs: events.append("commit")
        with patch.object(indexer, "write_record", side_effect=lambda *args: events.append("write")):
            indexer.process_record(self.consumer, Mock(), CONFIG, STREAM, record(8))
        self.assertEqual(events, ["write", "commit"])
        offsets = self.consumer.commit.call_args.kwargs["offsets"]
        self.assertEqual(offsets[self.tp].offset, 9)

    def test_failure_never_commits(self):
        with patch.object(indexer, "write_record", side_effect=TimeoutError()), self.assertRaises(TimeoutError):
            indexer.process_record(self.consumer, Mock(), CONFIG, STREAM, record())
        self.consumer.commit.assert_not_called()

    def test_lost_assignment_never_commits(self):
        self.consumer.assignment.side_effect = [{self.tp}, set()]
        with patch.object(indexer, "write_record"), self.assertRaises(RuntimeError):
            indexer.process_record(self.consumer, Mock(), CONFIG, STREAM, record())
        self.consumer.commit.assert_not_called()

    def test_unowned_record_never_writes(self):
        self.consumer.assignment.return_value = set()
        with patch.object(indexer, "write_record") as write, self.assertRaises(RuntimeError):
            indexer.process_record(self.consumer, Mock(), CONFIG, STREAM, record())
        write.assert_not_called()

    def test_commit_error_propagates_for_replay(self):
        self.consumer.commit.side_effect = RuntimeError("rebalance")
        with patch.object(indexer, "write_record"), self.assertRaises(RuntimeError):
            indexer.process_record(self.consumer, Mock(), CONFIG, STREAM, record())

    def test_invalid_checkpoints_require_resync(self):
        for committed, beginning, end in ((None, 2, 5), (1, 2, 5), (6, 0, 5)):
            with self.subTest(committed=committed), self.assertRaises(RuntimeError):
                indexer.validate_checkpoint(committed, beginning, end)

    def test_valid_checkpoints(self):
        for committed in (None, 0, 5):
            indexer.validate_checkpoint(committed, 0, 5)

    def test_failed_record_blocks_next_record_and_closes_without_commit(self):
        self.consumer.poll.return_value = {self.tp: [record(0), record(1)]}
        with patch.object(indexer, "describe_stream", return_value=STREAM), \
                patch.object(indexer, "set_role"), \
                patch.object(indexer, "write_record", side_effect=TimeoutError()) as write, \
                self.assertRaises(TimeoutError):
            indexer.consume(self.consumer, Mock(), Mock(), CONFIG, STREAM, stopped=lambda: False)
        self.assertEqual(write.call_count, 1)
        self.consumer.commit.assert_not_called()
        self.consumer.close.assert_called_once_with(autocommit=False)

    def test_recreated_topic_stops_before_write(self):
        self.consumer.poll.return_value = {self.tp: [record()]}
        with patch.object(indexer, "describe_stream", return_value={**STREAM, "topic_id": "other"}), \
                patch.object(indexer, "set_role"), patch.object(indexer, "write_record") as write, \
                self.assertRaises(RuntimeError):
            indexer.consume(self.consumer, Mock(), Mock(), CONFIG, STREAM, stopped=lambda: False)
        write.assert_not_called()
        self.consumer.commit.assert_not_called()

    def test_shutdown_does_not_poll_or_commit(self):
        with patch.object(indexer, "set_role"):
            indexer.consume(self.consumer, Mock(), Mock(), CONFIG, STREAM, stopped=lambda: True)
        self.consumer.poll.assert_not_called()
        self.consumer.commit.assert_not_called()
        self.consumer.close.assert_called_once_with(autocommit=False)

    def test_assignment_resumes_committed_offset(self):
        listener = indexer.AssignmentListener(self.consumer)
        self.assertTrue(inspect.iscoroutinefunction(listener.on_partitions_assigned))
        with patch.object(indexer, "describe_stream", return_value=STREAM), \
                patch.object(indexer, "read_checkpoints", return_value={self.tp: 8}), \
                patch.object(indexer, "set_role"):
            asyncio.run(listener.on_partitions_assigned([self.tp]))
        self.consumer.seek.assert_called_once_with(self.tp, 8)
        self.consumer.beginning_offsets.assert_not_called()
        self.consumer.end_offsets.assert_not_called()
        self.consumer.committed.assert_not_called()

    def test_revocation_never_commits(self):
        with patch.object(indexer, "set_role") as role:
            listener = indexer.AssignmentListener(self.consumer)
            self.assertTrue(inspect.iscoroutinefunction(listener.on_partitions_revoked))
            asyncio.run(listener.on_partitions_revoked([self.tp]))
        role.assert_called_once_with("standby")
        self.consumer.commit.assert_not_called()

    def test_async_checkpoint_error_leaves_standby_without_seek(self):
        listener = indexer.AssignmentListener(self.consumer)
        self.assertTrue(inspect.iscoroutinefunction(listener.on_partitions_assigned))
        with patch.object(indexer, "describe_stream", return_value=STREAM), \
                patch.object(indexer, "read_checkpoints", side_effect=RuntimeError("expired")), \
                patch.object(indexer, "set_role"), self.assertRaises(RuntimeError):
            asyncio.run(listener.on_partitions_assigned([self.tp]))
        self.consumer.seek.assert_not_called()

    def test_async_standby_does_not_read_checkpoints(self):
        listener = indexer.AssignmentListener(self.consumer)
        self.assertTrue(inspect.iscoroutinefunction(listener.on_partitions_assigned))
        with patch.object(indexer, "describe_stream", return_value=STREAM), \
                patch.object(indexer, "read_checkpoints") as read, patch.object(indexer, "set_role") as role:
            asyncio.run(listener.on_partitions_assigned([]))
        read.assert_not_called()
        role.assert_called_once_with("standby")

    def test_native_checkpoint_reader_awaits_fetch_and_coordinator(self):
        reader = Mock()
        reader._fetcher._fetch_offsets_by_times_async = AsyncMock(side_effect=[
            {self.tp: SimpleNamespace(offset=0)}, {self.tp: SimpleNamespace(offset=20)}])
        reader._coordinator.fetch_committed_offsets_async = AsyncMock(
            return_value={self.tp: SimpleNamespace(offset=8)})
        self.assertEqual(asyncio.run(indexer.read_checkpoints(reader, [self.tp])), {self.tp: 8})
        self.assertEqual(reader._fetcher._fetch_offsets_by_times_async.await_count, 2)
        reader._coordinator.fetch_committed_offsets_async.assert_awaited_once()
        reader.beginning_offsets.assert_not_called()
        reader.commit.assert_not_called()

    def test_native_expired_checkpoint_rejected(self):
        reader = Mock()
        reader._fetcher._fetch_offsets_by_times_async = AsyncMock(side_effect=[
            {self.tp: SimpleNamespace(offset=9)}, {self.tp: SimpleNamespace(offset=20)}])
        reader._coordinator.fetch_committed_offsets_async = AsyncMock(
            return_value={self.tp: SimpleNamespace(offset=8)})
        with self.assertRaises(RuntimeError):
            asyncio.run(indexer.read_checkpoints(reader, [self.tp]))


if __name__ == "__main__":
    unittest.main()
