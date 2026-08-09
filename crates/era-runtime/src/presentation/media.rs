use super::{PresentationModel, rgb_color};
use era_runtime_protocol::{
    AudioState, LogicalLength, MediaPlacement, RationalOpacity, ResourceReplay, TooltipFormat,
};

impl PresentationModel {
    pub(crate) fn play_bgm(&mut self, resource_id: String) {
        self.audio.clear();
        self.audio.push(AudioState {
            channel_id: 1,
            resource_id,
            repeat_count: -1,
            volume_millionths: 1_000_000,
            playing: true,
            revision: self.revision.saturating_add(1),
        });
        self.delivery.dirty.audio = true;
        self.bump();
    }

    pub(crate) fn stop_bgm(&mut self) {
        self.audio.clear();
        self.delivery.dirty.audio = true;
        self.bump();
    }

    pub(crate) fn set_bgm_volume(&mut self, volume: i64) {
        let volume = volume.clamp(0, 100);
        for state in &mut self.audio {
            state.volume_millionths = u32::try_from(volume).unwrap_or_default() * 10_000;
            state.revision = self.revision.saturating_add(1);
        }
        self.delivery.dirty.audio = true;
        self.bump();
    }

    pub(crate) fn projects_audio(&self) -> bool {
        self.project_audio
    }

    pub(crate) fn set_resource_replay(&mut self, resources: ResourceReplay) {
        self.resources = resources;
        self.resource_replay_stale = false;
        self.delivery.dirty.resources = true;
        self.bump();
    }

    pub(crate) fn mark_resource_replay_stale(&mut self) {
        self.resource_replay_stale = true;
    }

    pub(crate) const fn resource_replay_stale(&self) -> bool {
        self.resource_replay_stale
    }

    pub(crate) fn resource_replay_is_ready_to_publish(&self) -> bool {
        self.resource_replay_stale
            && (self.input_wait.is_some() || (self.redraw_enabled && self.delivery.dirty.redraw))
    }

    pub(crate) fn add_background(&mut self, resource_id: String, depth: i64, opacity: i64) {
        self.backgrounds.push(MediaPlacement {
            resource_id,
            x: LogicalLength(0),
            y: LogicalLength(0),
            width: self.settings.drawable_width,
            height: LogicalLength(0),
            depth,
            opacity: RationalOpacity {
                numerator: opacity,
                denominator: 255,
            },
            revision: self.revision.saturating_add(1),
            hover_resource_id: None,
            mask_resource_id: None,
            requested_width: None,
            requested_height: None,
            requested_y: None,
        });
        // Stable descending sort matches List.Sort's intended depth layering
        // while retaining insertion order for equal-depth portable replay.
        self.backgrounds
            .sort_by_key(|placement| std::cmp::Reverse(placement.depth));
        self.delivery.dirty.backgrounds = true;
        self.bump();
    }

    pub(crate) fn remove_background(&mut self, resource_id: &str) -> bool {
        let Some(index) = self
            .backgrounds
            .iter()
            .position(|item| item.resource_id == resource_id)
        else {
            return false;
        };
        self.backgrounds.remove(index);
        self.delivery.dirty.backgrounds = true;
        self.bump();
        true
    }

    pub(crate) fn clear_backgrounds(&mut self) {
        self.backgrounds.clear();
        self.delivery.dirty.backgrounds = true;
        self.bump();
    }

    pub(crate) fn clear_client_backgrounds(&mut self) {
        self.client_backgrounds.clear();
        self.delivery.dirty.backgrounds = true;
        self.bump();
    }

    pub(super) fn projected_backgrounds(&self) -> Vec<MediaPlacement> {
        let mut backgrounds =
            Vec::with_capacity(self.backgrounds.len() + self.client_backgrounds.len());
        backgrounds.extend(self.backgrounds.iter().cloned());
        backgrounds.extend(self.client_backgrounds.iter().cloned());
        backgrounds.sort_by_key(|placement| std::cmp::Reverse(placement.depth));
        backgrounds
    }

    pub(crate) fn set_tooltip_colors(&mut self, foreground: i64, background: i64) {
        self.tooltip.foreground = rgb_color(foreground);
        self.tooltip.background = rgb_color(background);
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_delay(&mut self, delay: i64) -> Result<(), &'static str> {
        self.tooltip.delay_ms =
            u32::try_from(delay).map_err(|_| "tooltip delay is out of range")?;
        self.delivery.dirty.tooltip = true;
        self.bump();
        Ok(())
    }

    pub(crate) fn set_tooltip_duration(&mut self, duration: i64) -> Result<(), &'static str> {
        let duration = u32::try_from(duration).map_err(|_| "tooltip duration is out of range")?;
        self.tooltip.duration_ms = duration.min(i16::MAX.cast_unsigned().into());
        self.delivery.dirty.tooltip = true;
        self.bump();
        Ok(())
    }

    pub(crate) fn set_tooltip_font(&mut self, family: String) {
        self.tooltip.font_family = (!family.is_empty()).then_some(family);
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_font_size(&mut self, points: i64) -> Result<(), &'static str> {
        let points = u32::try_from(points).map_err(|_| "tooltip font size is out of range")?;
        self.tooltip.font_millipoints = points.saturating_mul(1_000);
        self.delivery.dirty.tooltip = true;
        self.bump();
        Ok(())
    }

    pub(crate) fn set_tooltip_custom(&mut self, enabled: bool) {
        self.tooltip.custom = enabled;
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_format(&mut self, format: i64) {
        self.tooltip.format = format;
        self.tooltip.normalized_format = TooltipFormat::from_raw(format);
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_images(&mut self, enabled: bool) {
        self.tooltip.images = enabled;
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    #[allow(clippy::fn_params_excessive_bools)]
    pub(crate) fn set_projection(
        &mut self,
        column_cells: bool,
        separators: bool,
        html: bool,
        graphics: bool,
        audio: bool,
    ) {
        self.project_column_cells = column_cells;
        self.project_separators = separators;
        self.project_html = html;
        self.project_graphics = graphics;
        self.project_audio = audio;
        self.delivery.dirty.force_snapshot = true;
    }

    /// Clear runtime-owned content without forgetting capabilities negotiated by the client.
    pub(crate) fn reset_preserving_projection(&mut self) {
        let projection = (
            self.project_column_cells,
            self.project_separators,
            self.project_html,
            self.project_graphics,
            self.project_audio,
        );
        *self = Self::default();
        self.set_projection(
            projection.0,
            projection.1,
            projection.2,
            projection.3,
            projection.4,
        );
    }

    pub(crate) fn configure_project(
        &mut self,
        project: &crate::project::NormalizedProjectSnapshot,
    ) {
        use era_config::ConfigValue;

        let integer = |code| match project.configuration.get_code(code) {
            Some(ConfigValue::Integer(value)) => Some(*value),
            _ => None,
        };
        let boolean = |code| match project.configuration.get_code(code) {
            Some(ConfigValue::Boolean(value)) => Some(*value),
            _ => None,
        };
        let color = |code| match project.configuration.get_code(code) {
            Some(ConfigValue::Color(value)) => Some(*value),
            _ => None,
        };
        let font = match project.configuration.get_code("FontName") {
            Some(ConfigValue::String(value)) => Some(value.clone()),
            _ => None,
        };
        self.settings.drawable_width =
            LogicalLength(i64::from(project.viewport_width).saturating_mul(1_000));
        self.settings.drawable_height =
            LogicalLength(i64::from(project.viewport_height).saturating_mul(1_000));
        self.settings.line_height =
            LogicalLength(i64::from(project.line_height).saturating_mul(1_000));
        self.settings.maximum_physical_lines = integer("MaxLog")
            .and_then(|value| u32::try_from(value).ok())
            .map_or(5_000, |value| value.max(500));
        self.settings.prevent_button_wrap = boolean("ButtonWrap").unwrap_or(false);
        self.settings.legacy_nonbutton_wrap = boolean("CompatiLinefeedAs1739").unwrap_or(false);
        let mut default_style = self.default_style.clone();
        default_style.font_family = font;
        default_style.font_millipixels = project.font_size.saturating_mul(1_000);
        default_style.foreground = rgb_color(i64::from(color("ForeColor").unwrap_or(0x00c0_c0c0)));
        self.default_background = rgb_color(i64::from(color("BackColor").unwrap_or(0)));
        self.settings.background = self.default_background;
        self.settings.button_focus_foreground =
            rgb_color(i64::from(color("FocusColor").unwrap_or(0x00ff_ff00)));
        self.apply_project_default_style(default_style);
        self.print_c_length = project.print_c_length.max(1);
        self.trim_physical_history();
        self.delivery.dirty.settings = true;
        self.bump();
    }
}
