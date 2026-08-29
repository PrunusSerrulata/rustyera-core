//! Ordered devices and AWAIT acknowledgement reuse the ordinary service registry.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in crate::session) fn receive_device_state(
        &mut self,
        message_id: u64,
        state: era_runtime_protocol::DeviceStateChanged,
    ) -> Result<(), RuntimeError> {
        if let Err(message) = self.device_input.apply(&state) {
            return self.reject(message_id, CommandErrorCode::StaleRequest, message);
        }
        if self.phase == RuntimePhase::DebugPaused {
            self.debug_frontend_time_sample = Some(state.monotonic_time_ns);
        } else {
            self.observe_frontend_time(state.monotonic_time_ns);
            if state.device == era_runtime_protocol::InputDeviceKind::Mouse
                && state.code == 2
                && state.pressed
            {
                self.message_skip = true;
            }
        }
        Ok(())
    }

    pub(in crate::session) fn finish_device_pump(
        &mut self,
        request: erabasic_vm::HostRequestId,
        epoch: u64,
        after_event_sequence: u64,
        milliseconds: u64,
        response: ServiceResult,
    ) -> Result<(), RuntimeError> {
        let acknowledgement = match response {
            ServiceResult::Ready { payload } => {
                decode_canonical::<DevicePumpResponse>(payload.as_slice())
                    .map_err(|error| error.to_string())
            }
            ServiceResult::Error { error } => Err(format!("{}: {}", error.code, error.message)),
        };
        let acknowledgement = match acknowledgement {
            Ok(value)
                if value.epoch == epoch
                    && epoch == self.epoch.0
                    && value.through_event_sequence >= after_event_sequence
                    && value.through_event_sequence == self.device_input.event_sequence =>
            {
                value
            }
            Ok(_) => {
                return self.fault(
                    FaultCode::ServiceFailure,
                    "device pump acknowledgement precedes events or belongs to another epoch",
                    None,
                );
            }
            Err(message) => return self.fault(FaultCode::ServiceFailure, &message, None),
        };
        let _ = acknowledgement;
        if milliseconds != 0 {
            self.operations.insert_delay(
                request,
                self.logical_time_ns
                    .saturating_add(milliseconds.saturating_mul(1_000_000)),
            );
            return Ok(());
        }
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("device pump has no VM".into()))?;
        commit_completion(vm, request, VmHostCompletion::Ready(HostReady::empty()))?;
        self.set_phase(RuntimePhase::Running)
    }
}
