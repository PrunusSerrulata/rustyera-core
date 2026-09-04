//! SQL service emission and runtime-owned lifecycle cleanup.

#[allow(clippy::wildcard_imports)]
use super::super::super::*;

impl RuntimeSession {
    pub(in crate::session) fn emit_sql_cleanup_requests(&mut self) -> Result<u32, RuntimeError> {
        let provider = self.sql.provider();
        let handles = self.sql.cleanup_handles();
        self.emit_sql_cleanup_for(provider, &handles)
    }

    pub(in crate::session) fn emit_sql_cleanup_for(
        &mut self,
        provider: era_runtime_protocol::SqlProviderHandleV1,
        handles: &[era_runtime_protocol::SqlConnectionHandleV1],
    ) -> Result<u32, RuntimeError> {
        for connection in handles {
            self.retain_sql_cleanup(provider, *connection);
        }
        self.flush_sql_cleanup_queue()
    }

    pub(in crate::session) fn retain_sql_cleanup(
        &mut self,
        provider: era_runtime_protocol::SqlProviderHandleV1,
        connection: era_runtime_protocol::SqlConnectionHandleV1,
    ) {
        let cleanup = PendingSqlCleanup {
            provider,
            connection,
        };
        if !self.sql_cleanup_queue.contains(&cleanup) {
            self.sql_cleanup_queue.push(cleanup);
        }
    }

    pub(in crate::session) fn flush_sql_cleanup_queue(&mut self) -> Result<u32, RuntimeError> {
        let mut emitted = 0_u32;
        while let Some(cleanup) = self.sql_cleanup_queue.first().copied() {
            self.issue_sql_service_for(
                cleanup.provider,
                SqlServiceContinuation::CleanupDisconnect {
                    epoch: cleanup.provider.service_epoch,
                    provider: cleanup.provider,
                    connection: cleanup.connection,
                },
                era_runtime_protocol::SqlOperationV1::Disconnect {
                    connection: cleanup.connection,
                },
            )?;
            self.sql_cleanup_queue.remove(0);
            emitted = emitted.saturating_add(1);
        }
        Ok(emitted)
    }

    pub(super) fn issue_reserved_sql_service(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        connection_key: &str,
        continuation: SqlServiceContinuation,
        operation: era_runtime_protocol::SqlOperationV1,
    ) -> Result<(), RuntimeError> {
        if let Err(error) = self.begin_sql_host_request(vm, request) {
            self.sql.release_connection(connection_key);
            return Err(error);
        }
        let result = self.issue_sql_service(continuation, operation);
        if result.is_err() {
            self.sql.release_connection(connection_key);
        }
        result
    }

    pub(in crate::session) fn issue_sql_service(
        &mut self,
        continuation: SqlServiceContinuation,
        operation: era_runtime_protocol::SqlOperationV1,
    ) -> Result<(), RuntimeError> {
        self.issue_sql_service_for(self.sql.provider(), continuation, operation)
    }

    pub(in crate::session) fn issue_sql_service_for(
        &mut self,
        provider: era_runtime_protocol::SqlProviderHandleV1,
        continuation: SqlServiceContinuation,
        operation: era_runtime_protocol::SqlOperationV1,
    ) -> Result<(), RuntimeError> {
        let payload = era_runtime_protocol::SqlRequestV1 {
            provider,
            operation,
        };
        let payload = ProtocolBytes::new(encode_canonical(&payload)?);
        let request_id = self.allocate_request()?;
        self.operations
            .insert_service(request_id, PendingService::Sql(continuation));
        let result = self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind: ServiceKind::Sql,
                operation: SQL_OPERATION.into(),
                operation_version: SQL_OPERATION_VERSION,
                payload,
                deadline_ns: None,
            }),
            None,
        );
        if result.is_err() {
            self.operations.take_service(request_id);
        }
        result
    }

    pub(in crate::session) fn finish_sql_script_fault(
        &mut self,
        request: erabasic_vm::HostRequestId,
        kind: erabasic_vm::ScriptFaultKind,
        message: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("pending SQL request has no VM".into()))?;
        complete_script_fault_request(vm, request, kind, message)?;
        self.set_phase(RuntimePhase::Running)
    }
}
