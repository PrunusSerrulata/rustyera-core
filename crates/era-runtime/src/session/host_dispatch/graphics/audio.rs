#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    pub(super) fn dispatch_audio(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(name.as_str(), "PLAYBGM" | "PLAYSOUND") {
            *status = HostDispatchStatus::Handled;
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let bgm = name == "PLAYBGM";
            let resolved_resource = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.audio_path(&resource))
                .map(str::to_owned);
            let Some(resource) = resolved_resource else {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady::empty()),
                );
            };
            let revision = if bgm {
                self.presentation.play_bgm(resource.clone())
            } else {
                self.presentation.play_sound()
            };
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_bgm_presentation(bgm)?;
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(audio_effect(
                    bgm,
                    AudioEffectAction::Play,
                    Some(resource),
                    if bgm { -1 } else { 1 },
                    1_000_000,
                    revision,
                )));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if matches!(name.as_str(), "STOPBGM" | "STOPSOUND") {
            *status = HostDispatchStatus::Handled;
            let bgm = name == "STOPBGM";
            let revision = if bgm {
                self.presentation.stop_bgm()
            } else {
                self.presentation.stop_sound()
            };
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_bgm_presentation(bgm)?;
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(audio_effect(
                    bgm,
                    AudioEffectAction::Stop,
                    None,
                    0,
                    0,
                    revision,
                )));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if matches!(name.as_str(), "SETBGMVOLUME" | "SETSOUNDVOLUME") {
            *status = HostDispatchStatus::Handled;
            let bgm = name == "SETBGMVOLUME";
            let volume = integer_argument_value(&request.arguments, 0)?;
            let revision = if bgm {
                self.presentation.set_bgm_volume(volume)
            } else {
                self.presentation.set_sound_volume(volume)
            };
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_bgm_presentation(bgm)?;
            if self.presentation.projects_audio() && self.client_audio_available {
                let volume_millionths =
                    u32::try_from(volume.clamp(0, 100)).unwrap_or_default() * 10_000;
                return self.emit_effect(EffectKind::Audio(audio_effect(
                    bgm,
                    AudioEffectAction::SetVolume,
                    None,
                    0,
                    volume_millionths,
                    revision,
                )));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if name == "GETSOUNDORBGMINFO" {
            return self.dispatch_audio_info(vm, request, status);
        }

        Ok(())
    }

    fn dispatch_audio_info(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        *status = HostDispatchStatus::Handled;
        let channel = integer_argument_value(&request.arguments, 0)?;
        let Some(information) = request.arguments.get(1) else {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "GETSOUNDORBGMINFO without an information selector is not supported",
            );
        };
        let information = match information {
            VmValue::Integer(value) => *value,
            _ => {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Argument,
                    "GETSOUNDORBGMINFO argument 2 must be integer",
                );
            }
        };
        let value = self.presentation.sound_or_bgm_info(channel, information);
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Ready(HostReady {
                value: Some(VmValue::Integer(value)),
                writes: Vec::new(),
            }),
        )
    }

    fn emit_bgm_presentation(&mut self, bgm: bool) -> Result<(), RuntimeError> {
        if bgm {
            self.emit_presentation()?;
        }
        Ok(())
    }
}

fn audio_effect(
    bgm: bool,
    action: AudioEffectAction,
    resource_id: Option<String>,
    repeat_count: i64,
    volume_millionths: u32,
    revision: u64,
) -> AudioEffect {
    AudioEffect {
        channel: if bgm {
            AudioChannelV1::Bgm
        } else {
            AudioChannelV1::Sound(0)
        },
        action,
        resource_id,
        repeat_count,
        volume_millionths,
        revision,
        rate_millionths: 1_000_000,
        preserve_pitch: true,
    }
}
