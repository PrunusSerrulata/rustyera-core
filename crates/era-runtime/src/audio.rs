use era_runtime_protocol::{
    AUDIO_SOUND_CHANNEL_COUNT, AudioChannelV1, AudioEffect, AudioEffectAction,
    AudioObservationRequestV1,
};
use erabasic_vm::{HostRequestId, VmValue};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub(crate) struct AudioRuntimeState {
    sound_revisions: [u64; AUDIO_SOUND_CHANNEL_COUNT as usize],
    bgm_revision: u64,
    next_revision: u64,
    sound_observation_times: [u64; AUDIO_SOUND_CHANNEL_COUNT as usize],
    bgm_observation_time: u64,
}

impl Default for AudioRuntimeState {
    fn default() -> Self {
        Self {
            sound_revisions: [0; AUDIO_SOUND_CHANNEL_COUNT as usize],
            bgm_revision: 0,
            next_revision: 0,
            sound_observation_times: [0; AUDIO_SOUND_CHANNEL_COUNT as usize],
            bgm_observation_time: 0,
        }
    }
}

impl AudioRuntimeState {
    pub(crate) fn expected(&self, channel: AudioChannelV1) -> u64 {
        match channel {
            AudioChannelV1::Sound(channel) => self.sound_revisions[usize::from(channel)],
            AudioChannelV1::Bgm => self.bgm_revision,
        }
    }

    pub(crate) fn next_revision(&self, floor: u64) -> u64 {
        self.next_revision.max(floor).saturating_add(1)
    }

    pub(crate) fn commit_sound(&mut self, channel: u8, revision: u64) {
        self.sound_revisions[usize::from(channel)] = revision;
        self.next_revision = self.next_revision.max(revision);
    }

    pub(crate) fn commit_all_sounds(&mut self, revision: u64) {
        self.sound_revisions.fill(revision);
        self.next_revision = self.next_revision.max(revision);
    }

    pub(crate) fn commit_bgm(&mut self, revision: u64) {
        self.bgm_revision = revision;
        self.next_revision = self.next_revision.max(revision);
    }

    pub(crate) fn reset_transient(&mut self) {
        self.sound_revisions.fill(0);
        self.sound_observation_times.fill(0);
    }

    pub(crate) fn reset_all(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn recover_bgm(&mut self, revision: Option<u64>) {
        self.reset_transient();
        self.bgm_revision = revision.unwrap_or_default();
        self.bgm_observation_time = 0;
        self.next_revision = self.next_revision.max(self.bgm_revision);
    }

    pub(crate) fn record_observation(
        &mut self,
        channel: AudioChannelV1,
        frontend_monotonic_time_ns: u64,
    ) -> bool {
        let previous = match channel {
            AudioChannelV1::Sound(channel) => {
                &mut self.sound_observation_times[usize::from(channel)]
            }
            AudioChannelV1::Bgm => &mut self.bgm_observation_time,
        };
        if frontend_monotonic_time_ns < *previous {
            return false;
        }
        *previous = frontend_monotonic_time_ns;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AudioControl {
    Pause,
    Resume,
    Stop,
    SetRate {
        rate_millionths: u32,
        preserve_pitch: bool,
    },
}

impl AudioControl {
    pub(crate) fn parse(action: i64, arguments: &[VmValue], optional_start: usize) -> Option<Self> {
        if arguments.len() == optional_start {
            return match action {
                0 => Some(Self::Pause),
                1 => Some(Self::Resume),
                2 => Some(Self::Stop),
                _ => None,
            };
        }
        if action != 3 {
            return None;
        }
        let VmValue::Integer(speed_percent) = arguments.get(optional_start)? else {
            return None;
        };
        let preserve_pitch = arguments
            .get(optional_start + 1)
            .is_none_or(|value| matches!(value, VmValue::Integer(0)));
        Some(Self::SetRate {
            rate_millionths: u32::try_from((*speed_percent).clamp(10, 1_000)).unwrap_or(100)
                * 10_000,
            preserve_pitch,
        })
    }

    pub(crate) const fn protocol(self) -> (AudioEffectAction, u32, bool) {
        match self {
            Self::Pause => (AudioEffectAction::Pause, 1_000_000, true),
            Self::Resume => (AudioEffectAction::Resume, 1_000_000, true),
            Self::Stop => (AudioEffectAction::Stop, 1_000_000, true),
            Self::SetRate {
                rate_millionths,
                preserve_pitch,
            } => (AudioEffectAction::SetRate, rate_millionths, preserve_pitch),
        }
    }
}

pub(crate) fn play_effect(
    channel: AudioChannelV1,
    resource_id: String,
    repeat_count: i64,
    volume_millionths: u32,
    revision: u64,
) -> AudioEffect {
    AudioEffect {
        channel,
        action: AudioEffectAction::Play,
        resource_id: Some(resource_id),
        repeat_count,
        volume_millionths,
        revision,
        rate_millionths: 1_000_000,
        preserve_pitch: true,
    }
}

pub(crate) fn stop_effect(channel: AudioChannelV1, revision: u64) -> AudioEffect {
    control_effect(channel, AudioControl::Stop, 0, revision)
}

pub(crate) fn volume_effect(
    channel: AudioChannelV1,
    volume_millionths: u32,
    revision: u64,
) -> AudioEffect {
    AudioEffect {
        channel,
        action: AudioEffectAction::SetVolume,
        resource_id: None,
        repeat_count: 0,
        volume_millionths,
        revision,
        rate_millionths: 1_000_000,
        preserve_pitch: true,
    }
}

pub(crate) fn control_effect(
    channel: AudioChannelV1,
    control: AudioControl,
    volume_millionths: u32,
    revision: u64,
) -> AudioEffect {
    let (action, rate_millionths, preserve_pitch) = control.protocol();
    AudioEffect {
        channel,
        action,
        resource_id: None,
        repeat_count: 0,
        volume_millionths,
        revision,
        rate_millionths,
        preserve_pitch,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum AudioObservationPurpose {
    GetInfo {
        selector: Option<i64>,
    },
    IsPlayingSound,
    IsPlayingBgm,
    SelectSound {
        resource_id: String,
        repeat_count: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AudioObservationContinuation {
    pub(crate) request: HostRequestId,
    pub(crate) observation: AudioObservationRequestV1,
    pub(crate) purpose: AudioObservationPurpose,
}
