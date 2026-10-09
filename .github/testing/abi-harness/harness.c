/* CI ABI harness: the upstream header uses C++-style forward names; supply C declarations
 * without changing its definitions or ABI. */
typedef struct MongoExtensionAggStageParseNode MongoExtensionAggStageParseNode;
typedef struct MongoExtensionAggStageAstNode MongoExtensionAggStageAstNode;
typedef struct MongoExtensionLogicalAggStage MongoExtensionLogicalAggStage;
typedef struct MongoExtensionExecAggStage MongoExtensionExecAggStage;
typedef struct MongoExtensionDistributedPlanLogic MongoExtensionDistributedPlanLogic;
typedef struct MongoExtensionQueryExecutionContext MongoExtensionQueryExecutionContext;
#include "mongodb_extension_api.h"
#include <dlfcn.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define REQUIRE(condition) do { \
    if (!(condition)) { fprintf(stderr, "%s:%d: %s\n", __FILE__, __LINE__, #condition); exit(1); } \
} while (0)

static uint64_t (*abi_value)(const char *);
static MongoExtensionStatus *(*fixture_ok)(void);
static const MongoExtensionAggStageDescriptor *descriptor;

static void load_symbol(void *library, const char *name, void *out, size_t size) {
    void *symbol = dlsym(library, name);
    REQUIRE(symbol != NULL && size == sizeof(symbol));
    memcpy(out, &symbol, size);
}
#define LOAD(variable, name) load_symbol(library, name, &(variable), sizeof(variable))

static void status_ok(MongoExtensionStatus *status) {
    REQUIRE(status != NULL);
    REQUIRE(status->vtable->get_code(status) == MONGO_EXTENSION_STATUS_OK);
    status->vtable->destroy(status);
}

static void equal_view(MongoExtensionByteView view, const char *text) {
    REQUIRE(view.len == strlen(text));
    REQUIRE(memcmp(view.data, text, view.len) == 0);
}

static MongoExtensionStatus *register_descriptor(const MongoExtensionHostPortal *portal,
                                                const MongoExtensionAggStageDescriptor *stage) {
    (void)portal;
    REQUIRE(descriptor == NULL);
    descriptor = stage;
    return fixture_ok();
}

static MongoExtensionByteView options(const MongoExtensionHostPortal *portal) {
    (void)portal;
    static const uint8_t empty[] = "";
    return (MongoExtensionByteView){empty, 0};
}

static MongoExtensionStatus *register_rules(const MongoExtensionHostPortal *portal,
                                          MongoExtensionByteView stage,
                                          const MongoExtensionPipelineRewriteRule *rules,
                                          size_t count) {
    (void)portal; (void)stage; (void)rules; (void)count;
    return fixture_ok();
}

static void layouts(int negative) {
    size_t checks = 0;
#define CHECK(key, expression) do { \
    uint64_t rust = abi_value(key); \
    uint64_t c = (uint64_t)(expression); \
    if (negative && checks == 0) ++c; \
    if (rust != c) { fprintf(stderr, "ABI mismatch %s: Rust=%llu C=%llu\n", key, \
        (unsigned long long)rust, (unsigned long long)c); exit(1); } \
    ++checks; \
} while (0)
#include "layouts.h"
#undef CHECK
    REQUIRE(checks > 100);
    printf("%zu ABI size/alignment/offset/enum/version checks passed\n", checks);
}

static const MongoExtension *initialize(void *library) {
    get_mongodb_extension_versions_t versions;
    get_mongo_extension_t extension;
    LOAD(versions, "get_mongodb_extension_versions");
    LOAD(extension, "get_mongodb_extension");
    MongoExtensionAPIVersionVector supported = {0};
    versions(&supported);
    REQUIRE(supported.len == 1);
    REQUIRE(supported.versions[0].major == 1 && supported.versions[0].minor == 0);
    static const MongoExtensionHostServicesVTable services_vtable = {0};
    static const MongoExtensionHostServices services = {&services_vtable};
    const MongoExtension *result = NULL;
    MongoExtensionStatus *status = extension((MongoExtensionAPIVersion){99, 0}, &services, &result);
    REQUIRE(status->vtable->get_code(status) != MONGO_EXTENSION_STATUS_OK);
    REQUIRE(result == NULL);
    status->vtable->destroy(status);
    status_ok(extension(supported.versions[0], &services, &result));
    REQUIRE(result != NULL);
    static const MongoExtensionHostPortalVTable portal_vtable = {
        register_descriptor, options, register_rules
    };
    static const MongoExtensionHostPortal portal = {
        .vtable = &portal_vtable,
        .hostExtensionsAPIVersion = {1, 0},
        .hostMongoDBMaxWireVersion = 0,
    };
    status_ok(result->vtable->initialize(result, &portal));
    REQUIRE(descriptor != NULL);
    equal_view(descriptor->vtable->get_name(descriptor), "$abiHarness");
    return result;
}

static void lifecycle(void *library, int malformed) {
    (void)initialize(library);
    /* BSON { "$abiHarness": {} }, owned by the C caller. */
    static const uint8_t bson[] = {23, 0, 0, 0, 3, '$', 'a', 'b', 'i', 'H', 'a', 'r',
                                  'n', 'e', 's', 's', 0, 5, 0, 0, 0, 0, 0};
    MongoExtensionAggStageParseNode *parse = NULL;
    MongoExtensionStatus *status = descriptor->vtable->parse(
        descriptor, (MongoExtensionByteView){bson, malformed ? 3 : sizeof(bson)}, &parse);
    if (malformed) {
        REQUIRE(status->vtable->get_code(status) != MONGO_EXTENSION_STATUS_OK);
        REQUIRE(parse == NULL);
        REQUIRE(status->vtable->get_reason(status).len > 0);
        status->vtable->destroy(status);
        return;
    }
    status_ok(status);
    REQUIRE(parse != NULL);
    MongoExtensionAggStageParseNode *parse_clone = NULL;
    status_ok(parse->vtable->clone(parse, &parse_clone));
    equal_view(parse_clone->vtable->get_name(parse_clone), "$abiHarness");
    parse_clone->vtable->destroy(parse_clone);
    MongoExtensionByteBuf *serialized = NULL;
    status_ok(parse->vtable->to_bson_for_log(parse, &serialized));
    MongoExtensionByteView view = serialized->vtable->get_view(serialized);
    REQUIRE(view.len == sizeof(bson) && memcmp(view.data, bson, sizeof(bson)) == 0);
    serialized->vtable->destroy(serialized);
    MongoExtensionExpandedArrayContainer *container = NULL;
    status_ok(parse->vtable->expand(parse, &container));
    REQUIRE(container->vtable->size(container) == 1);
    MongoExtensionExpandedArrayElement element = {0};
    MongoExtensionExpandedArray array = {.size = 1, .elements = &element};
    status_ok(container->vtable->transfer(container, &array));
    container->vtable->destroy(container);
    REQUIRE(element.type == kAstNode);
    MongoExtensionAggStageAstNode *ast = element.parseOrAst.ast;
    MongoExtensionAggStageAstNode *ast_clone = NULL;
    status_ok(ast->vtable->clone(ast, &ast_clone));
    ast_clone->vtable->destroy(ast_clone);
    MongoExtensionCatalogContext context = {0};
    MongoExtensionLogicalAggStage *logical = NULL;
    status_ok(ast->vtable->bind(ast, &context, &logical));
    MongoExtensionLogicalAggStage *logical_clone = NULL;
    status_ok(logical->vtable->clone(logical, &logical_clone));
    logical_clone->vtable->destroy(logical_clone);
    status_ok(logical->vtable->serialize(logical, &serialized));
    serialized->vtable->destroy(serialized);
    MongoExtensionExecAggStage *exec = NULL;
    status_ok(logical->vtable->compile(logical, &exec));
    status_ok(exec->vtable->open(exec));
    for (size_t i = 0; i < 2; ++i) {
        MongoExtensionGetNextResult next = {0};
        status_ok(exec->vtable->get_next(exec, NULL, &next));
        REQUIRE(next.code == kEOF);
        REQUIRE(next.resultDocument.bytes.view.len == 0);
        REQUIRE(next.resultMetadata.bytes.view.len == 0);
    }
    status_ok(exec->vtable->reopen(exec));
    status_ok(exec->vtable->close(exec));
    exec->vtable->destroy(exec);
    logical->vtable->destroy(logical);
    ast->vtable->destroy(ast);
    parse->vtable->destroy(parse);
}

int main(int argc, char **argv) {
    REQUIRE(argc == 3);
    void *library = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (library == NULL) { fprintf(stderr, "%s\n", dlerror()); return 1; }
    LOAD(abi_value, "sdk_abi_value");
    LOAD(fixture_ok, "sdk_abi_ok");
    if (strcmp(argv[2], "layout") == 0) layouts(0);
    else if (strcmp(argv[2], "negative-layout") == 0) layouts(1);
    else if (strcmp(argv[2], "lifecycle") == 0) lifecycle(library, 0);
    else if (strcmp(argv[2], "malformed-bson") == 0) lifecycle(library, 1);
    else if (strcmp(argv[2], "ownership") == 0) {
        MongoExtensionByteBuf *(*buffer)(void);
        MongoExtensionStatus *(*error)(void);
        LOAD(buffer, "sdk_abi_buffer"); LOAD(error, "sdk_abi_error");
        MongoExtensionByteBuf *bytes = buffer();
        MongoExtensionByteView view = bytes->vtable->get_view(bytes);
        static const uint8_t expected[] = {1, 2, 3, 4};
        REQUIRE(view.len == sizeof(expected) && memcmp(view.data, expected, sizeof(expected)) == 0);
        bytes->vtable->destroy(bytes);
        MongoExtensionStatus *status = error();
        REQUIRE(status->vtable->get_code(status) == 42);
        equal_view(status->vtable->get_reason(status), "C ABI fixture error");
        status->vtable->destroy(status);
        status_ok(fixture_ok());
    } else if (strcmp(argv[2], "negative-ubsan") == 0) {
        volatile int maximum = INT_MAX;
        volatile int overflow = maximum + 1;
        (void)overflow;
    } else if (strcmp(argv[2], "negative-c-asan") == 0) {
        volatile unsigned char *bytes = malloc(1);
        REQUIRE(bytes != NULL);
        bytes[1] = 42;
        free((void *)bytes);
    } else if (strcmp(argv[2], "negative-rust-asan") == 0) {
        uint8_t (*probe)(void);
        LOAD(probe, "sdk_abi_asan_probe");
        volatile uint8_t value = probe();
        (void)value;
    } else { fprintf(stderr, "unknown case: %s\n", argv[2]); return 2; }
    /* SDK process-global registration remains reachable until process exit. */
    puts("PASS");
    return 0;
}
