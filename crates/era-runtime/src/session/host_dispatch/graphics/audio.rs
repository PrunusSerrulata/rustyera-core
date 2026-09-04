#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    pub(super) fn dispatch_audio(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        match name {
            "PLAYBGM" => self.dispatch_play_bgm(vm, request, status),
            "PLAYSOUND" => self.dispatch_play_sound(vm, request, status),
            "STOPBGM" | "STOPSOUND" => self.dispatch_stop_audio(vm, request, name, status),
            "SETBGMVOLUME" | "SETSOUNDVOLUME" => {
                self.dispatch_audio_volume(vm, request, name, status)
            }
            "GETSOUNDORBGMINFO" => self.dispatch_audio_info(vm, request, status),
            "ISPLAYINGSOUND" => self.dispatch_is_playing_sound(vm, request, status),
            "SOUNDCONTROL" => self.dispatch_sound_control(vm, request, status),
            "ISPLAYINGBGM" => self.dispatch_is_playing_bgm(vm, request, status),
            "BGMCONTROL" => self.dispatch_bgm_control(vm, request, status),
            _ => Ok(()),
        }
    }

    fn dispatch_play_bgm(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        *status = HostDispatchStatus::Handled;
        let Some(resource) = self.resolve_audio_resource(request) else {
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        };
        let revision = self.presentation.play_bgm(resource.clone());
        self.audio.commit_bgm(revision);
        commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
        self.emit_presentation()?;
        let _ = self.emit_audio_effects(vec![play_effect(
            AudioChannelV1::Bgm,
            resource,
            -1,
            1_000_000,
            revision,
        )])?;
        Ok(())
    }

    fn dispatch_play_sound(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        *status = HostDispatchStatus::Handled;
        let Some(resource_id) = self.resolve_audio_resource(request) else {
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        };
        let repeat_count = if request.arguments.len() > 1
            && request.omitted_arguments.binary_search(&1).is_err()
        {
            integer_argument_value(&request.arguments, 1)?.max(1)
        } else {
            1
        };
        if self.has_audio_observation_service() {
            return self.issue_audio_observation(
                vm,
                request,
                AudioChannelV1::Sound(0),
                AudioObservationPurpose::SelectSound {
                    resource_id,
                    repeat_count,
                },
            );
        }
        commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
        self.play_sound_on_channel(0, resource_id, repeat_count)
    }

    fn dispatch_stop_audio(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        *status = HostDispatchStatus::Handled;
        if name == "STOPBGM" {
            let revision = self.presentation.stop_bgm();
            self.audio.commit_bgm(revision);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            let _ = self.emit_audio_effects(vec![stop_effect(AudioChannelV1::Bgm, revision)])?;
            return Ok(());
        }
        let revision = self.audio.next_revision(0);
        commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
        let effects = (0..era_runtime_protocol::AUDIO_SOUND_CHANNEL_COUNT)
            .map(|channel| stop_effect(AudioChannelV1::Sound(channel), revision))
            .collect();
        if self.emit_audio_effects(effects)? {
            self.audio.commit_all_sounds(revision);
        }
        Ok(())
    }

    fn dispatch_audio_volume(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        *status = HostDispatchStatus::Handled;
        let volume = integer_argument_value(&request.arguments, 0)?;
        let volume_millionths = u32::try_from(volume.clamp(0, 100)).unwrap_or_default() * 10_000;
        if name == "SETBGMVOLUME" {
            let revision = self.presentation.set_bgm_volume(volume);
            self.audio.commit_bgm(revision);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            let _ = self.emit_audio_effects(vec![volume_effect(
                AudioChannelV1::Bgm,
                volume_millionths,
                revision,
            )])?;
            return Ok(());
        }
        let revision = self.presentation.set_sound_volume(volume);
        commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
        let effects = (0..era_runtime_protocol::AUDIO_SOUND_CHANNEL_COUNT)
            .map(|channel| {
                volume_effect(AudioChannelV1::Sound(channel), volume_millionths, revision)
            })
            .collect();
        if self.emit_audio_effects(effects)? {
            self.audio.commit_all_sounds(revision);
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
        let Some(channel) = audio_channel(integer_argument_value(&request.arguments, 0)?) else {
            return commit_integer_result(vm, request.id, 0);
        };
        let selector = if request.arguments.len() > 1
            && request.omitted_arguments.binary_search(&1).is_err()
        {
            Some(integer_argument_value(&request.arguments, 1)?)
        } else {
            None
        };
        if selector.is_some_and(|selector| !(1..=5).contains(&selector)) {
            return commit_integer_result(vm, request.id, 0);
        }
        self.issue_audio_observation(
            vm,
            request,
            channel,
            AudioObservationPurpose::GetInfo { selector },
        )
    }

    fn dispatch_is_playing_sound(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        *status = HostDispatchStatus::Handled;
        let Ok(channel) = u8::try_from(integer_argument_value(&request.arguments, 0)?) else {
            return commit_integer_result(vm, request.id, -1);
        };
        let Some(channel) = AudioChannelV1::sound(channel) else {
            return commit_integer_result(vm, request.id, -1);
        };
        self.issue_audio_observation(
            vm,
            request,
            channel,
            AudioObservationPurpose::IsPlayingSound,
        )
    }

    fn dispatch_is_playing_bgm(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        *status = HostDispatchStatus::Handled;
        self.issue_audio_observation(
            vm,
            request,
            AudioChannelV1::Bgm,
            AudioObservationPurpose::IsPlayingBgm,
        )
    }

    fn dispatch_sound_control(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        *status = HostDispatchStatus::Handled;
        let Ok(channel) = u8::try_from(integer_argument_value(&request.arguments, 0)?) else {
            return commit_integer_result(vm, request.id, -1);
        };
        let Some(target) = AudioChannelV1::sound(channel) else {
            return commit_integer_result(vm, request.id, -1);
        };
        let action = integer_argument_value(&request.arguments, 1)?;
        let Some(control) = AudioControl::parse(action, &request.arguments, 2) else {
            return commit_integer_result(vm, request.id, -2);
        };
        let revision = self.audio.next_revision(0);
        commit_integer_result(vm, request.id, 1)?;
        if self.emit_audio_effects(vec![control_effect(
            target,
            control,
            self.presentation.sound_volume_millionths(),
            revision,
        )])? {
            self.audio.commit_sound(channel, revision);
        }
        Ok(())
    }

    fn dispatch_bgm_control(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        *status = HostDispatchStatus::Handled;
        let action = integer_argument_value(&request.arguments, 0)?;
        let Some(control) = AudioControl::parse(action, &request.arguments, 1) else {
            return commit_integer_result(vm, request.id, -2);
        };
        let (protocol_action, rate_millionths, preserve_pitch) = control.protocol();
        let revision = if control == AudioControl::Stop {
            self.presentation.stop_bgm()
        } else {
            self.presentation
                .control_bgm(protocol_action, rate_millionths, preserve_pitch)
        };
        self.audio.commit_bgm(revision);
        commit_integer_result(vm, request.id, 1)?;
        self.emit_presentation()?;
        let _ = self.emit_audio_effects(vec![control_effect(
            AudioChannelV1::Bgm,
            control,
            0,
            revision,
        )])?;
        Ok(())
    }

    fn resolve_audio_resource(&self, request: &VmHostRequest) -> Option<String> {
        let resource = request
            .arguments
            .first()
            .map_or_else(String::new, display_value);
        self.project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.audio_path(&resource))
            .map(str::to_owned)
    }
}

fn audio_channel(channel: i64) -> Option<AudioChannelV1> {
    if channel == -1 {
        return Some(AudioChannelV1::Bgm);
    }
    u8::try_from(channel).ok().and_then(AudioChannelV1::sound)
}
