use super::*;
use crate::font_picker::FontTarget;

pub(super) fn view(s: &State, target: FontTarget, list_height: f32, open_above: bool) -> El {
    let picker = target.picker(s);
    let prefix = target.prefix();
    let selected = crate::preferences::font_display_name(&picker.selected);
    let selected_family = crate::preferences::installed_ui_fonts()
        .iter()
        .find(|font| font.eq_ignore_ascii_case(&picker.selected))
        .map_or(Family::Sans, |font| Family::Named(font.as_str()));
    let mut dropdown = col()
        .key(format!("{prefix}-control"))
        .grow(1.0)
        .min_w(0.0)
        .h(36.0)
        .child(
            row()
                .key(format!("{prefix}-dropdown"))
                .w_full()
                .h(36.0)
                .px(12.0)
                .gap(8.0)
                .center_y()
                .role("combobox")
                .label(format!("{} 선택", target.title()))
                .expanded(picker.open)
                .focusable(true)
                .focus_ring(1.5, 2.0, c("#78c4ce"))
                .fill(c("#111111"))
                .border(1.0, c("#343434"))
                .radius(4.0)
                .hover(c("#252525"))
                .on_press_tx(move |s: &mut State, tx| s.toggle_font_dropdown(target, tx))
                .on_key(move |s: &mut State, key, tx| {
                    if key.key == Key::Down {
                        s.toggle_font_dropdown(target, tx);
                        true
                    } else {
                        false
                    }
                })
                .child(
                    label(selected, 13.0, "#e6e6e6")
                        .font(TextStyle::new(13.0, 400.0).family(selected_family))
                        .grow(1.0)
                        .truncate(),
                )
                .child(glyph("chevron-down", 14.0).color(c("#aaaaaa"))),
        );
    if picker.open {
        let matches = picker.matches();
        let mut options = col()
            .key(format!("{prefix}-options-scroll"))
            .h(list_height)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .role("listbox")
            .label("설치된 글꼴 목록");
        for (index, font) in matches.iter().enumerate() {
            options = options.child(
                row()
                    .key(format!("{prefix}-option-{index}"))
                    .h(32.0)
                    .min_h(32.0)
                    .px(12.0)
                    .gap(8.0)
                    .center_y()
                    .role("option")
                    .label(*font)
                    .checked(*font == picker.selected)
                    .fill(c(if index == picker.highlighted {
                        "#34484c"
                    } else {
                        "#1e1e1e"
                    }))
                    .hover(c("#292929"))
                    .on_press_tx(move |s: &mut State, tx| s.select_font(target, index, tx))
                    .child(
                        label(crate::preferences::font_display_name(font), 14.0, "#e6e6e6")
                            .font(TextStyle::new(14.0, 400.0).family(Family::Named(font)))
                            .grow(1.0)
                            .truncate(),
                    )
                    .child(label(
                        if *font == picker.selected { "✓" } else { "" },
                        12.0,
                        "#78c4ce",
                    )),
            );
        }
        if matches.is_empty() {
            options = options.child(label("검색 결과가 없습니다.", 12.0, "#aaaaaa").padding(12.0));
        }
        dropdown = dropdown.child(
            col()
                .key(format!("{prefix}-popup"))
                .absolute(
                    0.0,
                    if open_above {
                        -(list_height + 76.0)
                    } else {
                        40.0
                    },
                )
                .w_full()
                .gap(0.0)
                .fill(c("#1e1e1e"))
                .border(1.0, c("#343434"))
                .radius(4.0)
                .child(
                    text_input(&picker.query, TextStyle::new(13.0, 400.0))
                        .key(format!("{prefix}-search"))
                        .label("글꼴 검색")
                        .single_line()
                        .w_full()
                        .h(40.0)
                        .min_h(40.0)
                        .padding(12.0)
                        .color(c("#e6e6e6"))
                        .placeholder("글꼴 검색", c("#8c8c8c"))
                        .on_edit(move |s: &mut State| &mut target.picker_mut(s).query)
                        .on_input_change(move |s: &mut State, tx| {
                            target.picker_mut(s).highlighted = 0;
                            tx.scroll_to_y(format!("{prefix}-options-scroll"), 0.0);
                        })
                        .on_input_key(move |s: &mut State, key, tx| {
                            s.font_dropdown_key(target, key, tx)
                        }),
                )
                .child(options)
                .child(
                    label(format!("{}개 글꼴", matches.len()), 12.0, "#8c8c8c")
                        .px(12.0)
                        .py(8.0),
                ),
        );
    }
    let setting = col()
        .key(format!("{prefix}-setting"))
        .gap(8.0)
        .paint_order(if picker.open { 20 } else { 10 })
        .child(label(target.title(), 13.0, "#e6e6e6"))
        .child(
            row()
                .key(format!("{prefix}-setting-row"))
                .w_full()
                .gap(8.0)
                .center_y()
                .child(dropdown)
                .child(
                    button(&format!("reset-{prefix}"), "초기화", |_| {})
                        .fill(Color::TRANSPARENT)
                        .border(0.0, Color::TRANSPARENT)
                        .on_press(move |s: &mut State| s.reset_font_selection(target))
                        .h(36.0),
                )
                .child(
                    button(&format!("apply-{prefix}"), "적용", |_| {})
                        .fill(c("#303030"))
                        .h(36.0)
                        .on_press_tx(move |s: &mut State, tx| match target {
                            FontTarget::Ui => s.apply_ui_font_selection(tx),
                            FontTarget::Editor => s.apply_editor_font_selection(tx),
                        }),
                ),
        );
    if picker.open {
        setting.modal_tx(true, move |s: &mut State, tx| {
            target.picker_mut(s).open = false;
            tx.focus(format!("{prefix}-dropdown"), false);
        })
    } else {
        setting
    }
}
