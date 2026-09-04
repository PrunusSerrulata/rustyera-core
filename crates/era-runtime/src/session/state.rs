#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionState {
    Negotiating,
    Active,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
enum SystemMenuState {
    #[default]
    Title,
    LoadSlots,
    SaveSlots,
    ConfirmOverwrite {
        slot: u32,
    },
}

#[derive(Debug)]
struct InboundStateTransfer {
    descriptor: StateTransferDescriptor,
    bytes: Vec<u8>,
    manifest_decoder: Option<transfer::state::manifest_import::ManifestImportDecoder>,
    hasher: Option<blake3::Hasher>,
    committed: bool,
}

impl InboundStateTransfer {
    fn received_bytes(&self) -> u64 {
        self.manifest_decoder
            .as_ref()
            .map_or(self.bytes.len() as u64, |decoder| decoder.received)
    }
}

#[derive(Debug)]
struct OutboundStateTransfer {
    descriptor: StateTransferDescriptor,
    bytes: OutboundBytes,
    next_offset: u64,
}

#[derive(Debug)]
enum OutboundBytes {
    Contiguous(Arc<Vec<u8>>),
    Container(Arc<crate::compiled_cache::ContainerBytes>),
}

impl OutboundBytes {
    fn len(&self) -> usize {
        match self {
            Self::Contiguous(bytes) => bytes.len(),
            Self::Container(bytes) => bytes.len(),
        }
    }
    fn copy_range(&self, range: std::ops::Range<usize>) -> Vec<u8> {
        match self {
            Self::Contiguous(bytes) => bytes[range].to_vec(),
            Self::Container(bytes) => bytes.copy_range(range),
        }
    }
}

#[derive(Debug)]
struct StagedFullProjectManifest {
    source_transfer_id: Option<u64>,
    manifest: ProjectManifest,
}

enum ProjectContainerTask {
    #[cfg(not(target_arch = "wasm32"))]
    Native {
        cancelled: Arc<AtomicBool>,
        handle: Option<JoinHandle<Result<Vec<u8>, String>>>,
    },
    #[cfg(any(target_arch = "wasm32", test))]
    Cooperative {
        encoder: Box<crate::compiled_cache::CooperativeCompiledCacheEncoder>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ProjectContainerTask {
    fn drop(&mut self) {
        match self {
            Self::Native {
                cancelled,
                handle: _,
            } => {
                cancelled.store(true, Ordering::Relaxed);
            }
            #[cfg(test)]
            Self::Cooperative { .. } => {}
        }
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum InboundMessage {
    Runtime(RuntimeMessage),
    Debug(DebugMessage),
}

#[derive(Clone, Debug)]
struct ActiveDebugGrant {
    token: GrantToken,
    scopes: BTreeSet<DebugScope>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectDiagnosticScope {
    artifact: [u8; 32],
    project_load_id: u64,
    runtime_epoch: u64,
    generation: Option<u64>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ProjectDiagnosticSite {
    code: String,
    source: Option<(String, u64, u64)>,
}

#[derive(Clone, Debug)]
struct ProjectDiagnosticPublication {
    scope: ProjectDiagnosticScope,
    sites: BTreeSet<ProjectDiagnosticSite>,
}

/// Single-owner runtime actor. Methods only enqueue, drive, and dequeue messages;
/// no frontend code can run inside a VM instruction dispatch.
#[allow(clippy::struct_excessive_bools)]
pub struct RuntimeSession {
    options: RuntimeOptions,
    project_progress_reporter: Option<ProjectProgressReporter>,
    state: SessionState,
    phase: RuntimePhase,
    revision: u64,
    epoch: SessionEpoch,
    expected_inbound_sequence: u64,
    expected_debug_sequence: u64,
    outbound_sequence: u64,
    debug_outbound_sequence: u64,
    next_message_id: u64,
    next_request_id: u64,
    next_wait_id: u64,
    next_interaction_id: u64,
    next_transfer_id: u64,
    next_effect_id: u64,
    logical_time_ns: u64,
    frontend_time_origin: Option<(u64, u64)>,
    random_seed: Option<u64>,
    input_replay: InputReplayHistory,
    next_new_game_trigger: NewGameTrigger,
    negotiated_features: BTreeSet<RuntimeFeature>,
    configuration_profile: ConfigurationClientProfile,
    client_preferences: Option<ClientPreferenceLayers>,
    inbound: VecDeque<(u64, Option<SessionEpoch>, InboundMessage)>,
    outbound: VecDeque<Vec<u8>>,
    outbound_journal: BTreeMap<u64, Vec<u8>>,
    outbound_journal_bytes: u64,
    effect_journal: BTreeMap<u64, EffectEvent>,
    accepted_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    accepted_debug_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    active_debug_grant: Option<ActiveDebugGrant>,
    next_debug_grant_id: u64,
    debug_resume_phase: Option<RuntimePhase>,
    debug_frontend_time_sample: Option<u64>,
    artifact: Option<ValidatedArtifact>,
    incremental: Arc<IncrementalState>,
    extension_declarations: Vec<ExtensionDeclaration>,
    vm: Option<RuntimeVm>,
    retained_title_program: Option<RetainedProgramIndex>,
    presentation: PresentationModel,
    audio: AudioRuntimeState,
    pending_presentation_update: bool,
    operations: PendingOperations,
    sql: SqlRuntimeState,
    sql_cleanup_queue: Vec<PendingSqlCleanup>,
    key_toggle_state: [u8; 256],
    device_input: crate::device_input::DeviceInput,
    environment: crate::environment::Environment,
    input_notice_sites: BTreeSet<(String, u64, erabasic_bytecode::SymbolKey, u32)>,
    hotkey_state: Vec<i64>,
    key_macros: KeyMacros,
    queued_input: VecDeque<QueuedInput>,
    input_controller: InputController,
    active_input_source: Option<InputSource>,
    deferred_input_completion: Option<InputSubmission>,
    text_box: String,
    text_box_layout: TextBoxLayout,
    flow_input_enabled: bool,
    flow_input_default: i64,
    flow_input_can_skip: bool,
    flow_input_force_skip: bool,
    flow_input_string: bool,
    flow_input_default_string: String,
    button_generation: u64,
    debug_output: String,
    debug_output_base: u64,
    debug_output_subscribed: bool,
    projection_environment_revision: u64,
    projection_space_revision: u64,
    last_projection_state: Option<ProjectionState>,
    client_width: u32,
    client_height: u32,
    line_columns: u32,
    message_skip: bool,
    skip_print: bool,
    user_defined_skip: bool,
    saved_skip: bool,
    force_kana_mode: u8,
    client_focused: bool,
    client_audio_available: bool,
    command_intents: BTreeMap<InteractionToken, VmValue>,
    reusable_system_intents: BTreeMap<InteractionToken, VmValue>,
    exit_requested: Option<ExitRequested>,
    controller: SystemController,
    undo_checkpoint: Option<UndoCheckpoint>,
    undo_replay: Option<UndoReplay>,
    undo_token: Option<InteractionToken>,
    project_snapshot: Option<NormalizedProjectSnapshot>,
    pending_configuration_update: Option<PendingConfigurationUpdate>,
    pending_sql_snapshot_restore: Option<PendingSqlSnapshotRestore>,
    ready_sql_snapshot_restore: Option<ReadySqlSnapshotRestore>,
    selected_locale: String,
    available_fonts: BTreeSet<String>,
    service_capabilities: BTreeMap<(ServiceKind, String), ProtocolVersion>,
    storage_capabilities: StorageCapabilities,
    save_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
    system_menu: SystemMenuState,
    load_slot_paths: Vec<String>,
    occupied_slot_paths: BTreeSet<String>,
    slot_change_tokens: BTreeMap<String, String>,
    slot_labels: BTreeMap<String, String>,
    invalid_slot_paths: BTreeSet<String>,
    system_menu_host_request: Option<erabasic_vm::HostRequestId>,
    system_menu_page: u32,
    inbound_transfer: Option<InboundStateTransfer>,
    outbound_transfer: Option<OutboundStateTransfer>,
    staged_project_manifest: Option<ProjectManifest>,
    staged_project_file_cache: Option<crate::compiled_cache::DecodedCompiledCache>,
    staged_full_project_manifest: Option<StagedFullProjectManifest>,
    pending_project_load: Option<PendingProjectLoad>,
    pending_candidate_commit: Option<PendingCandidateCommit>,
    candidate_clock: Option<LocalDateTimeResponse>,
    compiled_project_cache: Option<Arc<Vec<u8>>>,
    compiled_cache_diagnostics: Vec<ProtocolDiagnostic>,
    bitmap_cache_notice_emitted: bool,
    // Publication state is session-owned, never part of game snapshots or project caches.
    project_load_id: u64,
    project_diagnostic_publication: Option<ProjectDiagnosticPublication>,
    compiled_cache_task: Option<ProjectContainerTask>,
    compiled_cache_failure: Option<String>,
    full_project_file: Option<Arc<crate::compiled_cache::ContainerBytes>>,
    full_project_task: Option<ProjectContainerTask>,
    full_project_failure: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingSqlCleanup {
    provider: era_runtime_protocol::SqlProviderHandleV1,
    connection: era_runtime_protocol::SqlConnectionHandleV1,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct UndoCheckpoint {
    slot: u32,
    save_bytes: Vec<u8>,
    random_state: Vec<i64>,
    inputs: Vec<RecordedInput>,
    input_history_bytes: u64,
    input_controller: InputController,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct UndoReplay {
    remaining: VecDeque<RecordedInput>,
    queued_repeats: u32,
}

struct PendingProjectLoad {
    message_id: u64,
    remaining_metadata: BTreeSet<String>,
    queued_metadata: VecDeque<(String, [u8; 32])>,
    candidate: PendingProjectCandidate,
}

enum PendingProjectCandidate {
    Cold(PendingColdProjectLoad),
    Reload(PendingProjectReload),
}

impl PendingProjectCandidate {
    fn build_mut(&mut self) -> &mut crate::project::ProjectBuild {
        match self {
            Self::Cold(candidate) => &mut candidate.build,
            Self::Reload(candidate) => &mut candidate.build,
        }
    }
}

struct PendingColdProjectLoad {
    build: crate::project::ProjectBuild,
    previous_phase: RuntimePhase,
    compiled_project_cache: Option<Arc<Vec<u8>>>,
}

#[derive(Clone, Copy)]
enum SaveLoadScope {
    Ordinary,
    Global,
}

struct PendingProjectReload {
    build: crate::project::ProjectBuild,
    previous_phase: RuntimePhase,
    replay_origin: Option<ReplayOrigin>,
}

struct PendingConfigurationUpdate {
    preparation_message_id: u64,
    project_revision: u64,
    expected_source_digest: ProtocolBytes,
    prepared_source_digest: ProtocolBytes,
    contents: String,
    values: era_config::ConfigStore,
    document: era_config::ReraConfigDocument,
    changed_codes: BTreeSet<String>,
}

struct PendingSqlSnapshotRestore {
    message_id: u64,
    bytes: Vec<u8>,
    candidate_sql: SqlRuntimeState,
    remaining: VecDeque<crate::runtime_snapshot::SqlConnectionSnapshot>,
}

struct ReadySqlSnapshotRestore {
    digest: [u8; 32],
    candidate_sql: SqlRuntimeState,
}

#[allow(clippy::struct_excessive_bools)]
struct PendingCandidateCommit {
    state: PreparedCandidateState,
    presentation: PresentationModel,
    project_snapshot: Option<NormalizedProjectSnapshot>,
    message_skip: bool,
    skip_print: bool,
    user_defined_skip: bool,
    saved_skip: bool,
    force_kana_mode: u8,
    effects: Vec<EffectKind>,
    save_bytes: Vec<u8>,
    save_slot: Option<u32>,
}
