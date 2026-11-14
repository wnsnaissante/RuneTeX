use crate::{preferences, state::State};
use rune::{Key, KeyEvent, TextEdit, Tx};

#[derive(Clone, Copy)]
pub enum FontTarget {
    Ui,
    Editor,
}

impl FontTarget {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Ui => "ui-font",
            Self::Editor => "editor-font",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Ui => "UI 글꼴",
            Self::Editor => "에디터 글꼴",
        }
    }
    pub fn picker(self, state: &State) -> &FontPicker {
        match self {
            Self::Ui => &state.font_picker,
            Self::Editor => &state.editor_font_picker,
        }
    }
    pub fn picker_mut(self, state: &mut State) -> &mut FontPicker {
        match self {
            Self::Ui => &mut state.font_picker,
            Self::Editor => &mut state.editor_font_picker,
        }
    }
    pub fn default_font(self) -> &'static str {
        match self {
            Self::Ui => preferences::DEFAULT_UI_FONT,
            Self::Editor => preferences::DEFAULT_EDITOR_FONT,
        }
    }
}

pub struct FontPicker {
    pub open: bool,
    pub query: TextEdit,
    pub selected: String,
    pub highlighted: usize,
}

impl FontPicker {
    pub fn new(selected: String) -> Self {
        preferences::installed_ui_fonts();
        Self {
            open: false,
            query: TextEdit::default(),
            selected,
            highlighted: 0,
        }
    }
    pub fn matches(&self) -> Vec<&'static str> {
        let query: String = self
            .query
            .text
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        preferences::installed_ui_fonts()
            .iter()
            .filter(|name| {
                let name: String = preferences::font_display_name(name)
                    .to_lowercase()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                name.contains(&query)
            })
            .map(String::as_str)
            .collect()
    }
}

impl State {
    pub fn toggle_font_dropdown(&mut self, target: FontTarget, tx: &mut Tx) {
        let open = !target.picker(self).open;
        self.font_picker.open = false;
        self.editor_font_picker.open = false;
        let picker = target.picker_mut(self);
        picker.open = open;
        if open {
            tx.scroll_to_y("settings-scroll", 0.0);
            picker.query = TextEdit::default();
            picker.highlighted = picker
                .matches()
                .iter()
                .position(|font| *font == picker.selected)
                .unwrap_or(0);
            tx.focus(format!("{}-search", target.prefix()), false);
            tx.scroll_into_view(format!("{}-option-{}", target.prefix(), picker.highlighted));
        }
    }
    pub fn select_font(&mut self, target: FontTarget, index: usize, tx: &mut Tx) {
        let picker = target.picker_mut(self);
        if let Some(font) = picker.matches().get(index) {
            picker.selected = (*font).into();
            picker.open = false;
            tx.focus(format!("{}-dropdown", target.prefix()), false);
        }
    }
    pub fn reset_font_selection(&mut self, target: FontTarget) {
        let picker = target.picker_mut(self);
        picker.selected = target.default_font().into();
        picker.open = false;
    }
    pub fn font_dropdown_key(&mut self, target: FontTarget, key: &KeyEvent, tx: &mut Tx) -> bool {
        match key.key {
            Key::Escape => {
                target.picker_mut(self).open = false;
                tx.focus(format!("{}-dropdown", target.prefix()), false);
                true
            }
            Key::Enter => {
                self.select_font(target, target.picker(self).highlighted, tx);
                true
            }
            Key::Down | Key::Up => {
                let picker = target.picker_mut(self);
                let count = picker.matches().len();
                if count > 0 {
                    picker.highlighted = if key.key == Key::Down {
                        (picker.highlighted + 1).min(count - 1)
                    } else {
                        picker.highlighted.saturating_sub(1)
                    };
                    tx.scroll_into_view(format!(
                        "{}-option-{}",
                        target.prefix(),
                        picker.highlighted
                    ));
                }
                true
            }
            _ => false,
        }
    }
}
