//! Safe SQL host ABI translation. SQL remains an asynchronous frontend service; this module owns
//! validation and exposes only logical script handles.

#[allow(clippy::wildcard_imports)]
use super::super::*;

mod lifecycle;
mod storage;

impl RuntimeSession {
    pub(super) fn dispatch_sql(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        if self.candidate_clock.is_some() {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL operations are not allowed while preparing a candidate save",
            );
        }
        let name = request.import.import.name.to_ascii_uppercase();
        match name.as_str() {
            "SQL_CONNECT" => self.sql_connect(vm, request),
            "SQL_DISCONNECT" => self.sql_disconnect(vm, request),
            "SQL_EXECUTE_NONQUERY" | "SQL_P_EXECUTE_NONQUERY" => self.sql_execute(
                vm,
                request,
                era_runtime_protocol::SqlExecuteModeV1::NonQuery,
            ),
            "SQL_EXECUTE_SCALAR_LONG" | "SQL_P_EXECUTE_SCALAR_LONG" => self.sql_execute(
                vm,
                request,
                era_runtime_protocol::SqlExecuteModeV1::ScalarInteger,
            ),
            "SQL_EXECUTE_SCALAR_STRING" | "SQL_P_EXECUTE_SCALAR_STRING" => self.sql_execute(
                vm,
                request,
                era_runtime_protocol::SqlExecuteModeV1::ScalarString,
            ),
            "SQL_EXECUTE_READER" | "SQL_P_EXECUTE_READER" => {
                self.sql_execute(vm, request, era_runtime_protocol::SqlExecuteModeV1::Reader)
            }
            "SQL_READER_READ" => self.sql_reader_read(vm, request),
            "SQL_READER_GET_LONG" => self.sql_reader_get(vm, request, false),
            "SQL_READER_GET_STRING" => self.sql_reader_get(vm, request, true),
            "SQL_READER_ISNULL" => self.sql_reader_is_null(vm, request),
            "SQL_READER_CLOSE" => self.sql_reader_close(vm, request),
            "SQL_IMPORT_MAP_XML" => self.sql_import_map_xml(vm, request),
            _ => complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                format!("SQL API {name} is not implemented by rustyera.sql@1"),
            ),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn sql_connect(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let logical_name = string_argument_value(request, 0, "SQL_CONNECT")?;
        let Some(key) = crate::sql::normalize_sql_name(logical_name) else {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Argument,
                "SQL connection name must match ASCII [A-Za-z0-9_.-] and contain 1..64 bytes",
            );
        };
        let source = if request.omitted_arguments.binary_search(&1).is_ok()
            || request.arguments.get(1).is_none()
        {
            None
        } else {
            let value = string_argument_value(request, 1, "SQL_CONNECT")?;
            let Some(path) = crate::sql::parse_resource_connection_string(value) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Argument,
                    "SQL_CONNECT only accepts Data Source=<safe project Resource path>",
                );
            };
            Some(path)
        };
        if let Some(existing) = self.sql.connection_by_key(&key) {
            let same = match (&existing.identity.source, source.as_deref()) {
                (era_runtime_protocol::SqlDatabaseSourceV1::Memory, None) => true,
                (era_runtime_protocol::SqlDatabaseSourceV1::ResourceSeed(seed), Some(path)) => {
                    seed.resource_id.eq_ignore_ascii_case(path)
                }
                _ => false,
            };
            return if same {
                commit_integer_result(vm, request.id, 1)
            } else {
                complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    "SQL connection name is already bound to a different database source",
                )
            };
        }
        let resource = if let Some(path) = source.as_deref() {
            let Some(resource) = self.sql_project_resource(path) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Argument,
                    "SQL seed path is not an active project Resource",
                );
            };
            Some(resource)
        } else {
            None
        };
        if !self.require_host_service(
            request,
            ServiceKind::Sql,
            SQL_OPERATION,
            SQL_OPERATION_VERSION,
        )? {
            return Ok(());
        }
        let opening_source = resource
            .as_ref()
            .map_or(crate::sql::SqlOpeningSource::Memory, |(path, _)| {
                crate::sql::SqlOpeningSource::Resource(path.to_ascii_lowercase())
            });
        let connection = match self.sql.reserve_open(key.clone(), opening_source) {
            Ok(connection) => connection,
            Err(crate::sql::SqlOpenReservationError::Duplicate) => {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    "SQL connection open is already pending",
                );
            }
            Err(crate::sql::SqlOpenReservationError::Limit) => {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "SQL connection limit reached",
                );
            }
            Err(crate::sql::SqlOpenReservationError::Exhausted) => {
                return Err(RuntimeError::ResourceLimit(
                    "SQL connection handles exhausted",
                ));
            }
        };
        if let Err(error) = self.begin_sql_host_request(vm, request) {
            self.sql.release_open(&key, connection);
            return Err(error);
        }
        if let Some((path, expected_digest)) = resource {
            let result = self.issue_storage(
                PendingStorage::SqlSeedRead {
                    request: request.id,
                    epoch: self.sql.service_epoch(),
                    connection_key: key.clone(),
                    logical_name: logical_name.to_owned(),
                    connection,
                    path: path.clone(),
                    expected_digest,
                },
                StorageNamespace::Resource,
                StorageOperation::Read,
                path,
            );
            if result.is_err() {
                self.sql.release_open(&key, connection);
            }
            return result;
        }
        let identity = era_runtime_protocol::SqlDatabaseIdentityV1 {
            source: era_runtime_protocol::SqlDatabaseSourceV1::Memory,
            sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
            format_version: era_runtime_protocol::SQL_DATABASE_FORMAT_VERSION,
        };
        let result = self.issue_sql_service(
            SqlServiceContinuation::Open {
                request: request.id,
                epoch: self.sql.service_epoch(),
                logical_name: logical_name.to_owned(),
                connection,
                identity: identity.clone(),
                resource_digest: None,
            },
            era_runtime_protocol::SqlOperationV1::Open {
                connection,
                logical_name: logical_name.to_owned(),
                identity,
                revision: era_runtime_protocol::SqlOpenRevisionV1::Current,
                limits: era_runtime_protocol::SqlLimitsV1::FIXED,
            },
        );
        if result.is_err() {
            self.sql.release_open(&key, connection);
        }
        result
    }

    fn sql_execute(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        mode: era_runtime_protocol::SqlExecuteModeV1,
    ) -> Result<(), RuntimeError> {
        let name = string_argument_value(request, 0, &request.import.import.name)?;
        let Some(key) = crate::sql::normalize_sql_name(name) else {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Argument,
                "invalid SQL connection name",
            );
        };
        let sql_text = string_argument_value(request, 1, &request.import.import.name)?.to_owned();
        if sql_text.len() > era_runtime_protocol::SqlLimitsV1::FIXED.maximum_sql_bytes as usize {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Bounds,
                "SQL text exceeds the fixed 256 KiB limit",
            );
        }
        let Some(connection) = self.sql.connection_by_key(&key).map(|value| value.handle) else {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL connection is not open",
            );
        };
        let parameterized = request
            .import
            .import
            .name
            .to_ascii_uppercase()
            .starts_with("SQL_P_");
        let parameters = if parameterized {
            Self::sql_parameters(request, 2)?
        } else {
            Vec::new()
        };
        if !self.require_host_service(
            request,
            ServiceKind::Sql,
            SQL_OPERATION,
            SQL_OPERATION_VERSION,
        )? {
            return Ok(());
        }
        let reader_id = if mode == era_runtime_protocol::SqlExecuteModeV1::Reader {
            let Some(id) = self.sql.allocate_reader_id() else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "SQL reader limit reached",
                );
            };
            Some(id)
        } else {
            None
        };
        if !self.sql.reserve_connection(&key) {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL connection already has an operation in flight",
            );
        }
        self.issue_reserved_sql_service(
            vm,
            request,
            &key,
            SqlServiceContinuation::Execute {
                request: request.id,
                epoch: self.sql.service_epoch(),
                connection_key: key.clone(),
                connection,
                mode,
                reader_id,
            },
            era_runtime_protocol::SqlOperationV1::Execute {
                connection,
                mode,
                sql: sql_text,
                parameters,
            },
        )
    }

    fn sql_reader_read(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let id = integer_argument_value(request, 0)?;
        let Some(reader) = self.sql.reader(id).cloned() else {
            return commit_integer_result(vm, request.id, 0);
        };
        if matches!(
            reader.status,
            era_runtime_protocol::SqlReaderStatusV1::Eof
                | era_runtime_protocol::SqlReaderStatusV1::Closed
        ) {
            return commit_integer_result(vm, request.id, 0);
        }
        if !self.require_host_service(
            request,
            ServiceKind::Sql,
            SQL_OPERATION,
            SQL_OPERATION_VERSION,
        )? {
            return Ok(());
        }
        if !self.sql.reserve_connection(&reader.connection) {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL connection already has an operation in flight",
            );
        }
        self.issue_reserved_sql_service(
            vm,
            request,
            &reader.connection,
            SqlServiceContinuation::ReaderRead {
                request: request.id,
                epoch: self.sql.service_epoch(),
                reader_id: id,
                reader: reader.handle,
            },
            era_runtime_protocol::SqlOperationV1::ReaderRead {
                reader: reader.handle,
            },
        )
    }

    fn sql_reader_get(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        string: bool,
    ) -> Result<(), RuntimeError> {
        let id = integer_argument_value(request, 0)?;
        let column = integer_argument_value(request, 1)?;
        let Ok(column) = u32::try_from(column) else {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Bounds,
                "SQL reader column must be non-negative",
            );
        };
        let Some(reader) = self.sql.reader(id).cloned() else {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL reader is not open",
            );
        };
        if reader.status != era_runtime_protocol::SqlReaderStatusV1::Row {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL reader is not positioned on a row",
            );
        }
        if !self.require_host_service(
            request,
            ServiceKind::Sql,
            SQL_OPERATION,
            SQL_OPERATION_VERSION,
        )? {
            return Ok(());
        }
        if !self.sql.reserve_connection(&reader.connection) {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL connection already has an operation in flight",
            );
        }
        self.issue_reserved_sql_service(
            vm,
            request,
            &reader.connection,
            SqlServiceContinuation::ReaderGet {
                request: request.id,
                epoch: self.sql.service_epoch(),
                reader_id: id,
                reader: reader.handle,
                string,
            },
            era_runtime_protocol::SqlOperationV1::ReaderGet {
                reader: reader.handle,
                column,
                mode: if string {
                    era_runtime_protocol::SqlReaderValueModeV1::String
                } else {
                    era_runtime_protocol::SqlReaderValueModeV1::Integer
                },
            },
        )
    }

    fn sql_reader_is_null(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let id = integer_argument_value(request, 0)?;
        let Ok(column) = u32::try_from(integer_argument_value(request, 1)?) else {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Bounds,
                "SQL reader column must be non-negative",
            );
        };
        let Some(reader) = self.sql.reader(id).cloned() else {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL reader is not open",
            );
        };
        if !self.require_host_service(
            request,
            ServiceKind::Sql,
            SQL_OPERATION,
            SQL_OPERATION_VERSION,
        )? {
            return Ok(());
        }
        if reader.status != era_runtime_protocol::SqlReaderStatusV1::Row
            || !self.sql.reserve_connection(&reader.connection)
        {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL reader is not ready",
            );
        }
        self.issue_reserved_sql_service(
            vm,
            request,
            &reader.connection,
            SqlServiceContinuation::ReaderIsNull {
                request: request.id,
                epoch: self.sql.service_epoch(),
                reader_id: id,
                reader: reader.handle,
            },
            era_runtime_protocol::SqlOperationV1::ReaderIsNull {
                reader: reader.handle,
                column,
            },
        )
    }

    fn sql_reader_close(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let id = integer_argument_value(request, 0)?;
        let Some(reader) = self.sql.reader(id).cloned() else {
            return commit_integer_result(vm, request.id, 1);
        };
        if !self.require_host_service(
            request,
            ServiceKind::Sql,
            SQL_OPERATION,
            SQL_OPERATION_VERSION,
        )? {
            return Ok(());
        }
        if !self.sql.reserve_connection(&reader.connection) {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL connection already has an operation in flight",
            );
        }
        self.issue_reserved_sql_service(
            vm,
            request,
            &reader.connection,
            SqlServiceContinuation::ReaderClose {
                request: request.id,
                epoch: self.sql.service_epoch(),
                reader_id: id,
                reader: reader.handle,
            },
            era_runtime_protocol::SqlOperationV1::ReaderClose {
                reader: reader.handle,
            },
        )
    }

    fn sql_disconnect(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let name = string_argument_value(request, 0, "SQL_DISCONNECT")?;
        let Some(key) = crate::sql::normalize_sql_name(name) else {
            return commit_integer_result(vm, request.id, 1);
        };
        let Some(connection) = self.sql.connection_by_key(&key).map(|value| value.handle) else {
            return commit_integer_result(vm, request.id, 1);
        };
        if !self.require_host_service(
            request,
            ServiceKind::Sql,
            SQL_OPERATION,
            SQL_OPERATION_VERSION,
        )? {
            return Ok(());
        }
        if !self.sql.reserve_connection(&key) {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL connection cannot disconnect while an operation is pending",
            );
        }
        self.issue_reserved_sql_service(
            vm,
            request,
            &key,
            SqlServiceContinuation::Disconnect {
                request: request.id,
                epoch: self.sql.service_epoch(),
                connection_key: key.clone(),
                connection,
            },
            era_runtime_protocol::SqlOperationV1::Disconnect { connection },
        )
    }

    fn sql_import_map_xml(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let name = string_argument_value(request, 0, "SQL_IMPORT_MAP_XML")?;
        let Some(key) = crate::sql::normalize_sql_name(name) else {
            return commit_integer_result(vm, request.id, 0);
        };
        let Some(connection) = self.sql.connection_by_key(&key).map(|value| value.handle) else {
            return commit_integer_result(vm, request.id, 0);
        };
        let table = string_argument_value(request, 1, "SQL_IMPORT_MAP_XML")?.to_owned();
        if !crate::sql::validate_sql_table_name(&table) {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Argument,
                "SQL MAP table name must be a 1..64 byte ASCII identifier",
            );
        }
        let path = string_argument_value(request, 2, "SQL_IMPORT_MAP_XML")?;
        let Some((path, expected_digest)) = self.sql_project_resource(path) else {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Argument,
                "SQL MAP XML path is not an active project Resource",
            );
        };
        if !self.require_host_service(
            request,
            ServiceKind::Sql,
            SQL_OPERATION,
            SQL_OPERATION_VERSION,
        )? {
            return Ok(());
        }
        if !self.sql.reserve_connection(&key) {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "SQL connection already has an operation in flight",
            );
        }
        if let Err(error) = self.begin_sql_host_request(vm, request) {
            self.sql.release_connection(&key);
            return Err(error);
        }
        let result = self.issue_storage(
            PendingStorage::SqlMapXmlRead {
                request: request.id,
                epoch: self.sql.service_epoch(),
                connection_key: key.clone(),
                connection,
                table,
                path: path.clone(),
                expected_digest,
            },
            StorageNamespace::Resource,
            StorageOperation::Read,
            path,
        );
        if result.is_err() {
            self.sql.release_connection(&key);
        }
        result
    }

    fn begin_sql_host_request(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            },
        )?;
        self.set_phase(RuntimePhase::WaitingExternal)
    }

    fn sql_parameters(
        request: &VmHostRequest,
        start: usize,
    ) -> Result<Vec<era_runtime_protocol::SqlValueV1>, RuntimeError> {
        let count = request.arguments.get(start..).map_or(0, <[_]>::len);
        if count > era_runtime_protocol::SqlLimitsV1::FIXED.maximum_parameters as usize {
            return Err(RuntimeError::Script {
                kind: erabasic_vm::ScriptFaultKind::Bounds,
                message: "SQL parameter limit exceeded".into(),
            });
        }
        let mut bytes = 0_u64;
        let mut values = Vec::with_capacity(count);
        for index in start..request.arguments.len() {
            if request.omitted_arguments.binary_search(&index).is_ok() {
                values.push(era_runtime_protocol::SqlValueV1::Null);
                continue;
            }
            let value = display_value(request.arguments.get(index).expect("bounded argument"));
            bytes = bytes
                .checked_add(value.len() as u64)
                .ok_or_else(|| RuntimeError::Script {
                    kind: erabasic_vm::ScriptFaultKind::Bounds,
                    message: "SQL parameter bytes limit exceeded".into(),
                })?;
            values.push(era_runtime_protocol::SqlValueV1::String(value));
        }
        if bytes > era_runtime_protocol::SqlLimitsV1::FIXED.maximum_parameter_bytes {
            return Err(RuntimeError::Script {
                kind: erabasic_vm::ScriptFaultKind::Bounds,
                message: "SQL parameter bytes limit exceeded".into(),
            });
        }
        Ok(values)
    }

    fn sql_project_resource(&self, requested: &str) -> Option<(String, [u8; 32])> {
        let normalized = Self::resource_storage_path(requested, false)?;
        self.project_snapshot
            .as_ref()?
            .resources
            .iter()
            .find(|resource| {
                resource.category == FileCategory::Resource
                    && resource.relative_path.eq_ignore_ascii_case(&normalized)
            })
            .map(|resource| (resource.relative_path.clone(), resource.payload_digest))
    }
}
