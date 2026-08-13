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
            if bgm {
                self.presentation.play_bgm(resource.clone());
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            if bgm {
                self.emit_presentation()?;
            }
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(AudioEffect {
                    channel_id: u64::from(bgm),
                    action: AudioEffectAction::Play,
                    resource_id: Some(resource),
                    repeat_count: if bgm { -1 } else { 1 },
                    volume_millionths: 1_000_000,
                }));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if matches!(name.as_str(), "STOPBGM" | "STOPSOUND") {
            *status = HostDispatchStatus::Handled;
            let bgm = name == "STOPBGM";
            if bgm {
                self.presentation.stop_bgm();
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            if bgm {
                self.emit_presentation()?;
            }
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(AudioEffect {
                    channel_id: u64::from(bgm),
                    action: AudioEffectAction::Stop,
                    resource_id: None,
                    repeat_count: 0,
                    volume_millionths: 0,
                }));
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
            if bgm {
                self.presentation.set_bgm_volume(volume);
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            if bgm {
                self.emit_presentation()?;
            }
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(AudioEffect {
                    channel_id: u64::from(bgm),
                    action: AudioEffectAction::SetVolume,
                    resource_id: None,
                    repeat_count: 0,
                    volume_millionths: u32::try_from(volume.clamp(0, 100)).unwrap_or_default()
                        * 10_000,
                }));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }

        Ok(())
    }
}
