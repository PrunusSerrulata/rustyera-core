/* Checked C projection of the caller-pumped RustyEra runtime ABI. */
#ifndef ERA_RUNTIME_H
#define ERA_RUNTIME_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define ERA_RUNTIME_ABI_MAJOR 1u
#define ERA_RUNTIME_ABI_MINOR 0u

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
    void *reserved[8];
} EraRuntimeApi;

typedef EraStatus (*EraRuntimeGetApiFn)(EraAbiVersion, EraRuntimeApi *);
EraStatus era_runtime_get_api(EraAbiVersion requested, EraRuntimeApi *out_api);

#ifdef __cplusplus
}
#endif
#endif
