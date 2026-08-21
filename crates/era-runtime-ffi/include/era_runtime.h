/* Checked C projection of the caller-pumped RustyEra runtime ABI. */
#ifndef ERA_RUNTIME_H
#define ERA_RUNTIME_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define ERA_RUNTIME_ABI_MAJOR 3u
#define ERA_RUNTIME_ABI_MINOR 9u

#define ERA_DEBUG_SCOPE_VARIABLES_READ (UINT64_C(1) << 0)
#define ERA_DEBUG_SCOPE_VARIABLES_WRITE (UINT64_C(1) << 1)
#define ERA_DEBUG_SCOPE_GAME_FIELDS_READ (UINT64_C(1) << 2)
#define ERA_DEBUG_SCOPE_GAME_FIELDS_WRITE (UINT64_C(1) << 3)
#define ERA_DEBUG_SCOPE_EXECUTION_READ (UINT64_C(1) << 4)
#define ERA_DEBUG_SCOPE_EXECUTION_CONTROL (UINT64_C(1) << 5)
#define ERA_DEBUG_SCOPE_CONSOLE_EVALUATE (UINT64_C(1) << 6)
#define ERA_DEBUG_SCOPE_CONSOLE_EXECUTE (UINT64_C(1) << 7)
#define ERA_DEBUG_SCOPE_BREAKPOINTS_MANAGE (UINT64_C(1) << 8)
#define ERA_DEBUG_SCOPE_ALL ((UINT64_C(1) << 10) - 1)

typedef struct EraAbiVersion { uint16_t major; uint16_t minor; } EraAbiVersion;
typedef struct EraCallHeader { uint32_t struct_size; EraAbiVersion abi_version; } EraCallHeader;
typedef struct EraSessionHandle { uint64_t value; } EraSessionHandle;
typedef struct EraByteSlice { const uint8_t *data; size_t len; } EraByteSlice;
typedef struct EraOwnedBuffer { uint8_t *data; size_t len; uint64_t token; } EraOwnedBuffer;

typedef enum EraStatus {
    ERA_STATUS_OK = 0,
    ERA_STATUS_EMPTY = 1,
    ERA_STATUS_BUSY = 2,
    ERA_STATUS_INVALID_ARGUMENT = 3,
    ERA_STATUS_ABI_MISMATCH = 4,
    ERA_STATUS_INVALID_HANDLE = 5,
    ERA_STATUS_RESOURCE_LIMIT = 6,
    ERA_STATUS_INTERNAL_ERROR = 7
} EraStatus;

typedef struct EraCreateOptions {
    EraCallHeader header;
    uint64_t debug_scope_mask;
    uint64_t reserved[4];
} EraCreateOptions;

typedef struct EraDriveOptions {
    EraCallHeader header;
    uint64_t maximum_vm_instructions;
    uint32_t maximum_runtime_transitions;
    uint32_t reserved;
} EraDriveOptions;

typedef enum EraDriveState {
    ERA_DRIVE_IDLE = 0,
    ERA_DRIVE_MORE_WORK = 1,
    ERA_DRIVE_OUTPUT_READY = 2,
    ERA_DRIVE_STOPPED = 3,
    ERA_DRIVE_FAULTED = 4
} EraDriveState;

typedef struct EraDriveResult {
    EraCallHeader header;
    EraDriveState state;
    uint64_t vm_instructions;
    uint32_t runtime_transitions;
    uint32_t queued_envelopes;
} EraDriveResult;

typedef enum EraProjectProgressStage {
    ERA_PROJECT_PROGRESS_SCANNING = 0,
    ERA_PROJECT_PROGRESS_NORMALIZING = 1,
    ERA_PROJECT_PROGRESS_LOADING_DATA = 2,
    ERA_PROJECT_PROGRESS_PARSING = 3,
    ERA_PROJECT_PROGRESS_ANALYZING = 4,
    ERA_PROJECT_PROGRESS_COMPILING = 5,
    ERA_PROJECT_PROGRESS_VALIDATING = 6,
    ERA_PROJECT_PROGRESS_FINALIZING = 7,
    ERA_PROJECT_PROGRESS_PREPARING = 8,
    ERA_PROJECT_PROGRESS_PACKAGING = 9,
    ERA_PROJECT_PROGRESS_CACHE_PARSING = 10,
    ERA_PROJECT_PROGRESS_CACHE_DECODING = 11,
    ERA_PROJECT_PROGRESS_CACHE_VALIDATING = 12,
    ERA_PROJECT_PROGRESS_INITIALIZING_MEMORY = 13,
    ERA_PROJECT_PROGRESS_INDEXING_PROGRAM = 14
} EraProjectProgressStage;

typedef struct EraProjectProgress {
    EraCallHeader header;
    EraProjectProgressStage stage;
    uint64_t completed;
    uint64_t total;
} EraProjectProgress;

typedef void (*EraProjectProgressCallback)(void *context, EraProjectProgress progress);
typedef EraStatus (*EraSessionSetProjectProgressFn)(EraCallHeader, EraSessionHandle,
                                                     EraProjectProgressCallback, void *);
typedef EraStatus (*EraSessionDecodeProjectFileFn)(EraCallHeader, EraSessionHandle,
                                                    EraByteSlice, EraOwnedBuffer *);
typedef EraStatus (*EraPrepareProjectConfigurationUpdateFn)(
    EraCallHeader, EraSessionHandle, EraByteSlice, EraByteSlice, EraByteSlice,
    EraOwnedBuffer *);
typedef EraStatus (*EraSessionStageCompiledCacheFn)(EraCallHeader, EraSessionHandle,
                                                     EraByteSlice, uint64_t *);
/* ABI 3.5 writable cache ownership:
   - The caller must fill every returned byte before commit and must not access the buffer
     concurrently with commit/release.
   - Exactly one of commit or release_buffer must consume the original data/len/token triple.
   - A shape/handle/session-purpose rejection does not consume the buffer.
   - Once those checks pass, commit consumes the buffer even when it returns BUSY,
     RESOURCE_LIMIT, or INTERNAL_ERROR; the transfer id is written only on OK. */
typedef EraStatus (*EraSessionAllocateCompiledCacheFn)(EraCallHeader, EraSessionHandle,
                                                        size_t, EraOwnedBuffer *);
typedef EraStatus (*EraSessionCommitCompiledCacheFn)(EraCallHeader, EraSessionHandle,
                                                      EraOwnedBuffer, uint64_t *);
typedef EraStatus (*EraSessionStageProjectManifestFn)(EraCallHeader, EraSessionHandle,
                                                       EraByteSlice);

typedef EraStatus (*EraSessionCreateFn)(EraCallHeader, const EraCreateOptions *, EraSessionHandle *);
typedef EraStatus (*EraSessionSubmitFn)(EraCallHeader, EraSessionHandle, EraByteSlice);
typedef EraStatus (*EraSessionDriveFn)(EraCallHeader, EraSessionHandle, const EraDriveOptions *, EraDriveResult *);
typedef EraStatus (*EraSessionPollFn)(EraCallHeader, EraSessionHandle, EraOwnedBuffer *);
typedef EraStatus (*EraSessionDestroyFn)(EraCallHeader, EraSessionHandle);
typedef EraStatus (*EraReleaseBufferFn)(EraCallHeader, EraOwnedBuffer);
typedef EraStatus (*EraLastErrorFn)(EraCallHeader, EraSessionHandle, EraOwnedBuffer *);

typedef struct EraRuntimeApi {
    uint32_t struct_size;
    EraAbiVersion abi_version;
    const char *implementation_name;
    void *implementation_context;
    EraSessionCreateFn session_create;
    EraSessionSubmitFn session_submit;
    EraSessionDriveFn session_drive;
    EraSessionPollFn session_poll;
    EraSessionDestroyFn session_destroy;
    EraReleaseBufferFn release_buffer;
    EraLastErrorFn last_error;
    /* ABI 3.1: reserved[0] is EraSessionSetProjectProgressFn.
       ABI 3.2: reserved[1] is EraSessionDecodeProjectFileFn.
       ABI 3.3: reserved[2] is EraSessionDecodeProjectFileFn returning a compact frontend manifest.
       ABI 3.4: reserved[3] is EraSessionStageCompiledCacheFn.
       ABI 3.5: reserved[4] is EraSessionAllocateCompiledCacheFn and reserved[5] is
                EraSessionCommitCompiledCacheFn.
       ABI 3.6: reserved[6] is EraPrepareProjectConfigurationUpdateFn.
       ABI 3.8: reserved[7] is EraSessionStageProjectManifestFn. */
    void *reserved[8];
} EraRuntimeApi;

typedef EraStatus (*EraRuntimeGetApiFn)(EraAbiVersion, EraRuntimeApi *);
EraStatus era_runtime_get_api(EraAbiVersion requested, EraRuntimeApi *out_api);

#ifdef __cplusplus
}
#endif
#endif
