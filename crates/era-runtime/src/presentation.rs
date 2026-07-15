use era_runtime_protocol::{
    Color, DisplayLine, DisplayRun, InputWait, InteractionToken, LineAlignment,
    PresentationSettings, PresentationSnapshot, RunLayout, TextStyle,
};
use erabasic_vm::VmValue;

pub(crate) struct PresentationModel {
    revision: u64,
    title: String,
    lines: Vec<DisplayLine>,
    input_wait: Option<InputWait>,
    next_line: u64,
    settings: PresentationSettings,
}

impl Default for PresentationModel {
    fn default() -> Self {
        Self {
            revision: 0,
            title: String::new(),
            lines: Vec::new(),
            input_wait: None,
            next_line: 1,
            settings: PresentationSettings {
                drawable_width_millipixels: 760_000,
                line_height_millipixels: 19_000,
                background: Color {
                    red: 0,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                },
                button_focus_foreground: Color {
                    red: 255,
                    green: 255,
                    blue: 0,
                    alpha: 255,
                },
            },
        }
    }
}

impl PresentationModel {
    pub(crate) fn set_title(&mut self, title: String) {
        self.title = title;
        self.bump();
    }

    pub(crate) fn append_text(&mut self, text: String, temporary: bool) {
        let line = DisplayLine {
            line_id: self.next_line,
            temporary,
            logical_line_start: true,
            line_end: true,
            alignment: LineAlignment::Left,
            layout_width_millipixels: None,
            runs: vec![DisplayRun::Text {
                text,
                style: default_style(),
                layout: RunLayout {
                    x_millipixels: 0,
                    y_millipixels: 0,
                    width_millipixels: 0,
                    height_millipixels: self.settings.line_height_millipixels,
                    depth: 0,
                },
            }],
        };
        self.next_line = self.next_line.saturating_add(1);
        self.lines.push(line);
        self.bump();
    }

    pub(crate) fn append_button(&mut self, text: String, token: InteractionToken) {
        let layout = RunLayout {
            x_millipixels: 0,
            y_millipixels: 0,
            width_millipixels: 0,
            height_millipixels: self.settings.line_height_millipixels,
            depth: 0,
        };
        let line = DisplayLine {
            line_id: self.next_line,
            temporary: false,
            logical_line_start: true,
            line_end: true,
            alignment: LineAlignment::Left,
            layout_width_millipixels: None,
            runs: vec![DisplayRun::Button {
                runs: vec![DisplayRun::Text {
                    text,
                    style: default_style(),
                    layout,
                }],
                token,
                title: None,
                layout,
                hover_style: None,
            }],
        };
        self.next_line = self.next_line.saturating_add(1);
        self.lines.push(line);
        self.bump();
    }

    pub(crate) fn set_wait(&mut self, wait: Option<InputWait>) {
        self.input_wait = wait;
        self.bump();
    }

    pub(crate) fn snapshot(&self) -> PresentationSnapshot {
        PresentationSnapshot {
            revision: self.revision,
            title: self.title.clone(),
            lines: self.lines.clone(),
            backgrounds: Vec::new(),
            audio: Vec::new(),
            input_wait: self.input_wait.clone(),
            settings: self.settings.clone(),
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

pub(crate) fn display_value(value: &VmValue) -> String {
    match value {
        VmValue::Integer(value) => value.to_string(),
        VmValue::String(value) => value.clone(),
        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => "<place>".into(),
    }
}

fn default_style() -> TextStyle {
    TextStyle {
        foreground: Color {
            red: 192,
            green: 192,
            blue: 192,
            alpha: 255,
        },
        background: None,
        bold: false,
        italic: false,
        underline: false,
        strikeout: false,
        font_family: None,
        font_millipoints: 18_000,
    }
}
