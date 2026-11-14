mod font_picker;
mod settings;

use rune::{
    Color, Cx, El, Family, FileDialogOptions, Key, KeyEvent, ScrollSpec, TextStyle, Tx, col, el,
    row, text, text_input,
};

use crate::{
    compiler::Engine,
    icons::glyph,
    state::{DialogKind, State, ViewMode},
};

fn c(hex: &str) -> Color {
    rune::color::hex(hex).expect("valid theme color")
}
pub(crate) const CODE_FONT: Family = Family::Named(crate::preferences::CODE_FAMILIES);
fn label(value: impl Into<String>, size: f32, color: &str) -> El {
    text(value.into())
        .font(TextStyle::new(size, 400.0))
        .color(c(color))
}
fn mono(value: impl Into<String>, size: f32, color: &str) -> El {
    label(value, size, color).font(TextStyle::new(size, 400.0).family(CODE_FONT))
}
fn spacer() -> El {
    el().grow(1.0).min_w(0.0)
}
fn icon_button(key: &str, name: &str, help: &str, action: fn(&mut State)) -> El {
    row()
        .key(key)
        .size(32.0, 32.0)
        .center()
        .radius(4.0)
        .role("button")
        .label(help)
        .help(help)
        .focusable(true)
        .focus_ring(1.5, 2.0, c("#78c4ce"))
        .hover(c("#252525"))
        .on_press(action)
        .child(glyph(name, 16.0).color(c("#aaaaaa")))
}
fn button(key: &str, title: &str, action: fn(&mut State)) -> El {
    row()
        .key(key)
        .h(32.0)
        .px(12.0)
        .center()
        .gap(8.0)
        .radius(4.0)
        .fill(c("#1e1e1e"))
        .border(1.0, c("#343434"))
        .role("button")
        .label(title)
        .focusable(true)
        .focus_ring(1.5, 2.0, c("#78c4ce"))
        .hover(c("#292929"))
        .on_press(action)
        .child(label(title, 13.0, "#e6e6e6"))
}
fn keycap(value: &str) -> El {
    mono(value, 11.0, "#aaaaaa")
        .px(6.0)
        .py(2.0)
        .border(1.0, c("#343434"))
        .radius(4.0)
}

fn primary_button(key: &str, title: &str, action: fn(&mut State)) -> El {
    row()
        .key(key)
        .h(36.0)
        .px(16.0)
        .center()
        .radius(4.0)
        .fill(c("#e6e6e6"))
        .hover(c("#ffffff"))
        .role("button")
        .label(title)
        .focusable(true)
        .focus_ring(1.5, 2.0, c("#78c4ce"))
        .on_press(action)
        .child(label(title, 13.0, "#111111"))
}
fn open_search(s: &mut State, tx: &mut Tx) {
    s.dialog = Some(DialogKind::Search);
    tx.focus("search-input", false);
}
fn new_file(s: &mut State, tx: &mut Tx) {
    s.new_name = Default::default();
    s.dialog = Some(DialogKind::NewFile);
    tx.focus("new-name-input", false);
}
fn open_project(s: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "LaTeX 프로젝트 열기".into(),
        directory: Some(s.project_dir.clone()),
        ..Default::default()
    }
    .filter("LaTeX 문서", &["tex"]);
    tx.open_file(options, |s: &mut State, result, _| match result {
        Ok(Some(path)) => s.open_project(path),
        Err(error) => s.message = error.to_string(),
        _ => {}
    });
}
fn export_pdf(s: &mut State, tx: &mut Tx) {
    if s.latest_pdf.is_none() {
        s.message = "먼저 문서를 컴파일해 주세요.".into();
        return;
    }
    tx.save_file(
        FileDialogOptions {
            title: "PDF 내보내기".into(),
            file_name: Some("document.pdf".into()),
            directory: Some(s.project_dir.clone()),
            ..Default::default()
        }
        .filter("PDF", &["pdf"]),
        |s: &mut State, result, _| match result {
            Ok(Some(path)) => s.export_pdf(path),
            Err(error) => s.message = error.to_string(),
            _ => {}
        },
    );
}

pub fn on_key(s: &mut State, key: &KeyEvent, tx: &mut Tx) -> bool {
    if (key.mods.cmd || key.mods.ctrl) && !key.mods.alt {
        if key.is_command('+') || key.is_command('=') {
            s.change_editor_font_size(1.0);
            return true;
        }
        if key.is_command('-') {
            s.change_editor_font_size(-1.0);
            return true;
        }
    }
    if s.dialog.is_none() && s.language_key(key, tx) {
        return true;
    }
    if key.is_command('s') {
        s.save_all();
        return true;
    }
    if key.is_command('k') {
        open_search(s, tx);
        return true;
    }
    if key.is_command('b') {
        s.sidebar = !s.sidebar;
        return true;
    }
    if key.is_command('o') {
        open_project(s, tx);
        return true;
    }
    if key.is_command('n') {
        new_file(s, tx);
        return true;
    }
    if key.is_command('w') && s.dialog.is_none() {
        s.close_tab(s.active);
        return true;
    }
    if key.mods.cmd && key.key == Key::Enter {
        s.start_compile();
        return true;
    }
    if key.key == Key::Escape {
        if s.dialog.take().is_none() {
            tx.focus("compile-button", false);
        }
        return true;
    }
    false
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let width = cx.layout_size("window").map_or(1440.0, |size| size[0]);
    let height = cx.layout_size("window").map_or(940.0, |size| size[1]);
    let sidebar_width = if width < 1100.0 { 210.0 } else { 248.0 };
    let available = width - if s.sidebar { sidebar_width } else { 0.0 };
    let split = s.mode == ViewMode::Split;
    let pane_width = if split { available - 8.0 } else { available };
    let min_ratio = (240.0 / pane_width.max(1.0)).min(0.45);
    let editor_width = if split {
        pane_width * s.split_ratio.clamp(min_ratio, 1.0 - min_ratio)
    } else {
        available
    };
    let mut work = row()
        .key("workbench")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(0.0);
    if s.sidebar {
        work = work.child(sidebar(s, sidebar_width));
    }
    if s.mode != ViewMode::Preview {
        work = work.child(editor(s).w(editor_width));
    }
    if split {
        work = work.child(
            row()
                .key("pane-divider")
                .w(8.0)
                .h_full()
                .center()
                .fill(c("#111111"))
                .hover(c("#34484c"))
                .cursor(rune::Cursor::ResizeColumn)
                .role("separator")
                .label("소스와 PDF 너비 조절")
                .help("드래그로 너비 조절 · 방향키로 이동 · Home으로 초기화")
                .focusable(true)
                .focus_ring(1.5, 0.0, c("#78c4ce"))
                .on_drag(move |s: &mut State, drag, _| match drag.phase {
                    rune::Phase::Down => {
                        s.split_ratio = s.split_ratio.clamp(min_ratio, 1.0 - min_ratio);
                        drag.data = s.split_ratio;
                    }
                    rune::Phase::Move => {
                        s.split_ratio =
                            (drag.data + drag.dx / pane_width).clamp(min_ratio, 1.0 - min_ratio);
                    }
                    rune::Phase::Up => {
                        if drag.cancelled() {
                            s.split_ratio = drag.data;
                        }
                    }
                })
                .on_key(move |s: &mut State, key, _| {
                    match key.key {
                        Key::Left => {
                            s.split_ratio = (s.split_ratio.clamp(min_ratio, 1.0 - min_ratio) - 0.02)
                                .clamp(min_ratio, 1.0 - min_ratio)
                        }
                        Key::Right => {
                            s.split_ratio = (s.split_ratio.clamp(min_ratio, 1.0 - min_ratio) + 0.02)
                                .clamp(min_ratio, 1.0 - min_ratio)
                        }
                        Key::Home => s.split_ratio = 0.52,
                        _ => return false,
                    }
                    true
                })
                .child(el().w(2.0).h(24.0).radius(1.0).fill(c("#414141"))),
        );
    }
    if s.mode != ViewMode::Source {
        work = work.child(preview(
            s,
            if split {
                pane_width - editor_width
            } else {
                available
            },
        ));
    }
    let mut window = col()
        .key("window")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(c("#080808"))
        .child(topbar(s, if s.sidebar { sidebar_width } else { 224.0 }))
        .child(work)
        .child(statusbar(s));
    if s.dialog.is_some() {
        window = window.child(dialog(s, height));
    } else if !s.language.completions.is_empty() || s.language.information.is_some() {
        window = window.child(language_popup(s, cx, width));
    }
    window
}

fn topbar(s: &State, sidebar_width: f32) -> El {
    let search = row()
        .key("quick-open")
        .grow(1.0)
        .max_w(400.0)
        .min_w(160.0)
        .h(32.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .fill(c("#111111"))
        .border(1.0, c("#272727"))
        .radius(4.0)
        .role("button")
        .label("프로젝트 검색")
        .focusable(true)
        .hover(c("#1e1e1e"))
        .on_press_tx(open_search)
        .child(glyph("search", 16.0).color(c("#8c8c8c")))
        .child(label("프로젝트 검색", 13.0, "#aaaaaa"))
        .child(spacer())
        .child(keycap("Ctrl K"));
    let compile = row()
        .key("compile-button")
        .h(32.0)
        .px(12.0)
        .center_y()
        .gap(8.0)
        .fill(c(if s.compiling { "#292929" } else { "#e6e6e6" }))
        .radius(4.0)
        .role("button")
        .label("컴파일")
        .help("PDF 컴파일 · Ctrl Enter")
        .focusable(true)
        .disabled(s.compiling)
        .focus_ring(1.5, 2.0, c("#78c4ce"))
        .hover(c("#ffffff"))
        .on_press(|s: &mut State| s.start_compile())
        .child(
            glyph(if s.compiling { "clock" } else { "play" }, 16.0).color(c(if s.compiling {
                "#aaaaaa"
            } else {
                "#111111"
            })),
        )
        .child(label(
            "컴파일",
            13.0,
            if s.compiling { "#aaaaaa" } else { "#111111" },
        ))
        .child(
            mono(
                "Ctrl ↵",
                11.0,
                if s.compiling { "#8c8c8c" } else { "#565656" },
            )
            .ml(4.0),
        );
    row()
        .key("topbar")
        .h(48.0)
        .w_full()
        .px(16.0)
        .gap(16.0)
        .center_y()
        .fill(c("#080808"))
        .border_sides([0.0, 0.0, 1.0, 0.0], c("#272727"))
        .child(
            row()
                .w(sidebar_width - 16.0)
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(icon_button(
                    "toggle-sidebar",
                    "panel-left",
                    "탐색기 열기·닫기 · Ctrl B",
                    |s| s.sidebar = !s.sidebar,
                ))
                .child(
                    row()
                        .key("open-project")
                        .grow(1.0)
                        .min_w(0.0)
                        .h(32.0)
                        .px(4.0)
                        .gap(8.0)
                        .center_y()
                        .radius(4.0)
                        .hover(c("#252525"))
                        .role("button")
                        .label("프로젝트 열기")
                        .help("프로젝트 열기 · Ctrl O")
                        .focusable(true)
                        .focus_ring(1.5, 2.0, c("#78c4ce"))
                        .on_press_tx(open_project)
                        .child(glyph("folder", 16.0).color(c("#8c8c8c")))
                        .child(
                            label(s.project_name(), 13.0, "#e6e6e6")
                                .grow(1.0)
                                .min_w(0.0)
                                .truncate(),
                        ),
                ),
        )
        .child(search)
        .child(spacer())
        .child(
            row()
                .center_y()
                .gap(8.0)
                .child(icon_button(
                    "settings",
                    "sliders",
                    "편집기 설정",
                    |s| {
                        s.font_picker.open = false;
                        s.font_picker.selected = s.ui_font.clone();
                        s.editor_font_picker.open = false;
                        s.editor_font_picker.selected = s.editor_font.clone();
                        s.dialog = Some(DialogKind::Settings);
                    },
                ))
                .child(el().w(1.0).h(20.0).mx(4.0).fill(c("#272727")))
                .child(compile),
        )
}

fn sidebar(s: &State, width: f32) -> El {
    let mut tree = col().key("file-tree").w_full().px(8.0).gap(0.0);
    let mut last_folder = String::new();
    for (index, file) in s.files.iter().enumerate() {
        let folder = file.path.rsplit_once('/').map(|(f, _)| f).unwrap_or("");
        if !folder.is_empty() && folder != last_folder {
            tree = tree.child(
                row()
                    .h(32.0)
                    .px(8.0)
                    .gap(8.0)
                    .center_y()
                    .child(glyph("chevron-down", 12.0).color(c("#8c8c8c")))
                    .child(glyph("folder", 16.0).color(c("#8c8c8c")))
                    .child(label(folder, 13.0, "#aaaaaa")),
            );
        }
        last_folder = folder.to_string();
        let active = index == s.active;
        let name = file.name();
        tree = tree.child(
            row()
                .key(format!("file-{index}"))
                .w_full()
                .h(32.0)
                .pl(if folder.is_empty() { 8.0 } else { 40.0 })
                .pr(8.0)
                .gap(8.0)
                .center_y()
                .radius(2.0)
                .fill(c(if active { "#292929" } else { "#111111" }))
                .hover(c("#252525"))
                .role("button")
                .label(&file.path)
                .help(&file.path)
                .focusable(true)
                .on_press_tx(move |s: &mut State, tx| {
                    s.switch_file(index);
                    tx.focus("source-editor", false);
                })
                .child(
                    glyph(
                        if name.ends_with(".bib") {
                            "book"
                        } else {
                            "file"
                        },
                        16.0,
                    )
                    .color(c(if active { "#e6e6e6" } else { "#8c8c8c" })),
                )
                .child(label(name, 13.0, if active { "#e6e6e6" } else { "#aaaaaa" }).grow(1.0))
                .child(label(
                    if file.edit.text != file.saved_text {
                        "•"
                    } else {
                        ""
                    },
                    12.0,
                    "#78c4ce",
                )),
        );
    }
    let mut outline = col().key("outline-list").px(8.0);
    for (i, item) in s.outline().iter().enumerate() {
        let file = item.file;
        let line = item.line;
        outline = outline.child(
            row()
                .key(format!("outline-{i}"))
                .min_h(32.0)
                .pl(8.0 + item.level as f32 * 16.0)
                .pr(8.0)
                .gap(8.0)
                .center_y()
                .radius(2.0)
                .role("button")
                .label(if item.number.is_empty() {
                    item.title.clone()
                } else {
                    format!("{} {}", item.number, item.title)
                })
                .focusable(true)
                .hover(c("#1e1e1e"))
                .on_press_tx(move |s: &mut State, tx| s.jump_to(file, line, tx))
                .child(mono(&item.number, 12.0, "#8c8c8c").min_w(16.0))
                .child(label(&item.title, 13.0, "#aaaaaa").grow(1.0).truncate()),
        );
    }
    col()
        .key("sidebar")
        .w(width)
        .h_full()
        .min_h(0.0)
        .fill(c("#111111"))
        .border_sides([0.0, 1.0, 0.0, 0.0], c("#272727"))
        .child(
            row()
                .h(40.0)
                .px(16.0)
                .center_y()
                .justify_between()
                .child(label("파일", 13.0, "#8c8c8c"))
                .child(
                    icon_button("new-file", "plus", "새 파일 · Ctrl N", |_| {})
                        .size(24.0, 24.0)
                        .on_press_tx(new_file),
                ),
        )
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(tree)
                .child(
                    row()
                        .mt(20.0)
                        .h(40.0)
                        .px(16.0)
                        .gap(8.0)
                        .center_y()
                        .border_sides([1.0, 0.0, 0.0, 0.0], c("#272727"))
                        .child(glyph("chevron-down", 12.0).color(c("#8c8c8c")))
                        .child(label("문서 개요", 13.0, "#aaaaaa")),
                )
                .child(outline),
        )
}

fn modes(s: &State) -> El {
    let mut group = row().gap(4.0).center_y();
    for (mode, name, title) in [
        (ViewMode::Source, "code", "소스만 보기"),
        (ViewMode::Split, "split", "나란히 보기"),
        (ViewMode::Preview, "eye", "미리보기만 보기"),
    ] {
        group = group.child(
            row()
                .key(format!("mode-{name}"))
                .size(28.0, 24.0)
                .center()
                .radius(4.0)
                .fill(c(if s.mode == mode { "#292929" } else { "#0c0c0c" }))
                .role("button")
                .label(title)
                .help(title)
                .focusable(true)
                .hover(c("#252525"))
                .on_press(move |s: &mut State| s.mode = mode)
                .child(glyph(name, 14.0).color(c(if s.mode == mode {
                    "#e6e6e6"
                } else {
                    "#8c8c8c"
                }))),
        );
    }
    group
}

fn editor(s: &State) -> El {
    let mut tabs = row()
        .key("tab-list")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .scroll_x();
    for &index in &s.tabs {
        let file = &s.files[index];
        let active = index == s.active;
        tabs = tabs.child(
            row()
                .key(format!("tab-{index}"))
                .h(40.0)
                .min_w(132.0)
                .px(12.0)
                .gap(12.0)
                .center_y()
                .fill(c(if active { "#0c0c0c" } else { "#101010" }))
                .border_sides([0.0, 1.0, 0.0, 0.0], c("#272727"))
                .child(
                    row()
                        .key(format!("select-tab-{index}"))
                        .grow(1.0)
                        .h_full()
                        .center_y()
                        .gap(8.0)
                        .role("tab")
                        .label(&file.path)
                        .focusable(true)
                        .on_press_tx(move |s: &mut State, tx| {
                            s.switch_file(index);
                            tx.focus("source-editor", false);
                        })
                        .child(glyph("file", 14.0).color(c("#8c8c8c")))
                        .child(mono(
                            file.name(),
                            12.0,
                            if active { "#e6e6e6" } else { "#8c8c8c" },
                        )),
                )
                .child(
                    row()
                        .key(format!("close-tab-{index}"))
                        .size(20.0, 20.0)
                        .center()
                        .radius(3.0)
                        .hover(c("#292929"))
                        .role("button")
                        .label("탭 닫기")
                        .focusable(true)
                        .on_press(move |s: &mut State| s.close_tab(index))
                        .child(if file.edit.text != file.saved_text {
                            label("•", 14.0, "#78c4ce")
                        } else {
                            glyph("x", 12.0).color(c("#8c8c8c"))
                        }),
                ),
        );
    }
    let source = &s.active_edit().text;
    let line_height = s.font_size * 1.5;
    let code_style = TextStyle::new(s.font_size, 400.0)
        .family(Family::Mono)
        .lh(line_height);
    let content_width = rune::text::measure(source, &[], code_style, None)
        .0
        .max(100.0)
        + 24.0;
    let line_count = source.bytes().filter(|b| *b == b'\n').count() + 1;
    let gutter_width =
        (rune::text::measure(&line_count.to_string(), &[], code_style, None).0 + 24.0).max(48.0);
    let line_numbers = (1..=line_count)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let input = text_input(s.active_edit(), code_style)
        .key("source-editor")
        .label(format!("{} 소스 편집", s.active_file().path))
        .color(c("#e6e6e6"))
        .caret(c("#78c4ce"), c("#34484c"))
        .min_w(100.0)
        .min_h(400.0)
        .pr(24.0)
        .pb(64.0)
        .on_edit(|s: &mut State| s.active_edit_mut())
        .on_input_change(|s: &mut State, _| s.on_edit())
        .on_input_key(|s: &mut State, key, tx| {
            if on_key(s, key, tx) {
                return true;
            }
            if key.key == Key::Tab && !key.mods.shift {
                s.active_edit_mut().insert("  ");
                s.on_edit();
                return true;
            }
            if key.key == Key::Enter {
                let edit = s.active_edit_mut();
                let prefix = &edit.text[..edit.focus];
                let indent: String = prefix
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                edit.insert(&format!("\n{indent}"));
                s.on_edit();
                return true;
            }
            false
        })
        .spans(s.editor_spans())
        // A zero-width fade disables soft wrapping without enabling single-line editing.
        .truncate_fade_width(0.0)
        .w(content_width)
        .min_w(content_width);
    col()
        .key("editor-pane")
        .h_full()
        .min_h(0.0)
        .min_w(0.0)
        .fill(c("#0c0c0c"))
        .border_sides([0.0, 1.0, 0.0, 0.0], c("#272727"))
        .child(
            row()
                .h(40.0)
                .w_full()
                .min_w(0.0)
                .fill(c("#101010"))
                .border_sides([0.0, 0.0, 1.0, 0.0], c("#272727"))
                .child(tabs)
                .child(
                    icon_button("new-tab", "plus", "새 파일", |_| {})
                        .mx(8.0)
                        .on_press_tx(new_file),
                ),
        )
        .child(
            row()
                .h(32.0)
                .w_full()
                .px(16.0)
                .center_y()
                .gap(8.0)
                .child(
                    label(
                        format!(
                            "{}  /  {}",
                            s.project_dir
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                            s.active_file().path
                        ),
                        11.0,
                        "#8c8c8c",
                    )
                    .grow(1.0)
                    .min_w(0.0),
                )
                .child(modes(s)),
        )
        .child(
            col()
                .key("source-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll_xy()
                .child(
                    row()
                        .key("code-content")
                        .pt(12.0)
                        .min_w(0.0)
                        .child(
                            mono(line_numbers, s.font_size, "#8c8c8c")
                                .font(
                                    TextStyle::new(s.font_size, 400.0)
                                        .family(Family::Mono)
                                        .lh(line_height),
                                )
                                .w(gutter_width)
                                .pr(16.0)
                                .align(rune::Align::End)
                                .a11y_hidden(),
                        )
                        .child(input),
                ),
        )
        .child(output_panel(s))
}

fn output_panel(s: &State) -> El {
    let diagnostics = s.all_diagnostics();
    let errors = diagnostics.iter().filter(|d| !d.warning).count();
    let mut panel = col()
        .key("output-panel")
        .fill(c("#0c0c0c"))
        .border_sides([1.0, 0.0, 0.0, 0.0], c("#272727"))
        .child(
            row()
                .h(32.0)
                .px(16.0)
                .center_y()
                .gap(20.0)
                .child(
                    row()
                        .key("show-problems")
                        .h_full()
                        .center_y()
                        .gap(8.0)
                        .role("tab")
                        .label("문제")
                        .focusable(true)
                        .on_press(|s: &mut State| {
                            s.panel_log = false;
                            s.panel_open = true;
                        })
                        .child(label(
                            "문제",
                            12.0,
                            if s.panel_log { "#8c8c8c" } else { "#e6e6e6" },
                        ))
                        .child(mono(
                            errors.to_string(),
                            11.0,
                            if errors > 0 { "#ef9393" } else { "#8c8c8c" },
                        )),
                )
                .child(
                    row()
                        .key("show-log")
                        .h_full()
                        .center_y()
                        .role("tab")
                        .label("출력")
                        .focusable(true)
                        .on_press(|s: &mut State| {
                            s.panel_log = true;
                            s.panel_open = true;
                        })
                        .child(label(
                            "출력",
                            12.0,
                            if s.panel_log { "#e6e6e6" } else { "#8c8c8c" },
                        )),
                )
                .child(spacer())
                .child(
                    icon_button(
                        "toggle-panel",
                        if s.panel_open {
                            "chevron-down"
                        } else {
                            "chevron-up"
                        },
                        "출력 패널 펼치기·접기",
                        |s| s.panel_open = !s.panel_open,
                    )
                    .size(24.0, 24.0),
                ),
        );
    if s.panel_open {
        let mut content = col()
            .key("panel-content")
            .h(160.0)
            .px(20.0)
            .py(12.0)
            .gap(8.0)
            .scroll_xy();
        if s.panel_log {
            content = content.child(
                mono(
                    if s.log.is_empty() {
                        "컴파일하면 엔진 출력이 여기에 표시됩니다."
                    } else {
                        &s.log
                    },
                    11.0,
                    "#aaaaaa",
                )
                .selectable(true),
            );
        } else if diagnostics.is_empty() {
            content = content.child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(glyph("circle-check", 16.0).color(c("#b7d99a")))
                    .child(label("발견된 문제가 없습니다.", 12.0, "#aaaaaa")),
            );
        } else {
            for (index, diagnostic) in diagnostics.iter().enumerate() {
                let location = diagnostic.file.clone().unwrap_or_default();
                let line = diagnostic.line;
                let target = s.files.iter().position(|f| location.ends_with(&f.path));
                content = content.child(
                    row()
                        .key(format!("diagnostic-{index}"))
                        .gap(8.0)
                        .min_h(24.0)
                        .center_y()
                        .role("button")
                        .label(&diagnostic.message)
                        .focusable(true)
                        .hover(c("#1e1e1e"))
                        .on_press_tx(move |s: &mut State, tx| {
                            if let Some(file) = target {
                                s.jump_to(file, line.unwrap_or(1), tx);
                            }
                        })
                        .child(glyph("alert", 14.0).color(c(if diagnostic.warning {
                            "#e0ba80"
                        } else {
                            "#ef9393"
                        })))
                        .child(
                            label(
                                format!(
                                    "{}{}  {}",
                                    location,
                                    line.map(|l| format!(":{l}")).unwrap_or_default(),
                                    diagnostic.message
                                ),
                                11.0,
                                "#aaaaaa",
                            )
                            .grow(1.0),
                        ),
                );
            }
        }
        panel = panel.child(content);
    }
    panel
}

fn preview(s: &State, width: f32) -> El {
    let mut pages = col()
        .key("preview-pages")
        .gap(24.0)
        .items_center()
        .padding(24.0);
    if s.preview_pages.is_empty() {
        pages = pages.child(
            col()
                .key("preview-empty")
                .w_full()
                .min_h(540.0)
                .padding(40.0)
                .center()
                .gap(16.0)
                .child(glyph("file", 32.0).color(c("#656565")))
                .child(label(
                    if s.compiling {
                        "문서를 컴파일하고 있습니다"
                    } else {
                        "문서를 미리 볼 준비가 되었습니다"
                    },
                    16.0,
                    "#e6e6e6",
                ))
                .child(label(
                    "Ctrl Enter로 LaTeX를 PDF로 컴파일하세요.",
                    12.0,
                    "#8c8c8c",
                ))
                .child(
                    button("empty-compile", "컴파일", |s| s.start_compile())
                        .mt(8.0)
                        .disabled(s.compiling),
                ),
        );
    } else {
        for (index, pixels) in s.preview_pages.iter().enumerate() {
            let (pw, ph) = pixels.size();
            let page_width = ((width - 48.0).min(640.0) * s.zoom).max(100.0);
            pages = pages.child(
                el().key(format!("pdf-page-{index}"))
                    .w(page_width)
                    .h(page_width * ph as f32 / pw.max(1) as f32)
                    .fill(Color::WHITE)
                    .pixels(pixels)
                    .role("img")
                    .label(format!("컴파일된 PDF 페이지 {}", index + 1)),
            );
        }
    }
    col()
        .key("preview-pane")
        .w(width)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .fill(c("#161616"))
        .child(
            row()
                .h(40.0)
                .px(16.0)
                .center_y()
                .gap(10.0)
                .fill(c("#101010"))
                .border_sides([0.0, 0.0, 1.0, 0.0], c("#272727"))
                .child(label("미리보기", 13.0, "#aaaaaa"))
                .child(spacer())
                .child(label(
                    if s.preview_stale() && !s.preview_pages.is_empty() {
                        "이전 PDF"
                    } else if !s.preview_pages.is_empty() {
                        "최신 PDF"
                    } else {
                        ""
                    },
                    12.0,
                    if s.preview_stale() {
                        "#e0ba80"
                    } else {
                        "#8c8c8c"
                    },
                ))
                .child(
                    icon_button("export-pdf", "download", "PDF 내보내기", |_| {})
                        .size(24.0, 24.0)
                        .on_press_tx(export_pdf)
                        .disabled(s.latest_pdf.is_none()),
                )
                .child(
                    icon_button("preview-back", "split", "소스와 나란히 보기", |s| {
                        s.mode = ViewMode::Split
                    })
                    .size(24.0, 24.0),
                ),
        )
        .child(
            row()
                .h(32.0)
                .px(16.0)
                .center_y()
                .gap(8.0)
                .fill(c("#111111"))
                .border_sides([0.0, 0.0, 1.0, 0.0], c("#272727"))
                .child(label(
                    format!("{} 페이지", s.preview_pages.len()),
                    12.0,
                    "#8c8c8c",
                ))
                .child(spacer())
                .child(
                    icon_button("zoom-out", "minus", "미리보기 축소", |s| {
                        s.zoom = (s.zoom - 0.1).max(0.3)
                    })
                    .size(24.0, 24.0),
                )
                .child(
                    button("zoom-reset", &format!("{:.0}%", s.zoom * 100.0), |s| {
                        s.zoom = 1.0
                    })
                    .h(24.0)
                    .border(0.0, Color::TRANSPARENT)
                    .fill(c("#111111")),
                )
                .child(
                    icon_button("zoom-in", "plus", "미리보기 확대", |s| {
                        s.zoom = (s.zoom + 0.1).min(3.0)
                    })
                    .size(24.0, 24.0),
                ),
        )
        .child(
            col()
                .key("preview-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll_xy()
                .child(pages),
        )
}

fn language_popup(s: &State, cx: &mut Cx<State>, width: f32) -> El {
    let caret = cx
        .caret("source-editor")
        .unwrap_or([300.0, 160.0, 1.0, 24.0]);
    let height = cx.layout_size("window").map_or(940.0, |size| size[1]);
    let popup_height = if s.language.information.is_some() {
        300.0
    } else {
        250.0
    };
    let x = caret[0].clamp(8.0, (width - 390.0).max(8.0));
    let below = caret[1] + caret[3] + 3.0;
    let y = if below + popup_height > height - 28.0 {
        (caret[1] - popup_height).max(48.0)
    } else {
        below
    };
    let mut popup = col()
        .key("language-popup")
        .absolute(x, y)
        .w(380.0)
        .max_h(popup_height)
        .fill(c("#1e1e1e"))
        .border(1.0, c("#414141"))
        .radius(4.0)
        .gap(0.0);
    if let Some(information) = &s.language.information {
        popup = popup
            .child(
                row()
                    .h(30.0)
                    .px(10.0)
                    .center_y()
                    .justify_between()
                    .child(label("texlab · 명령 설명", 11.0, "#78c4ce"))
                    .child(
                        icon_button("close-language-info", "x", "설명 닫기", |s| {
                            s.language.information = None
                        })
                        .size(24.0, 24.0),
                    ),
            )
            .child(
                col()
                    .h(260.0)
                    .min_h(0.0)
                    .padding(10.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(label(information, 12.0, "#e6e6e6").wrap().selectable(true)),
            );
    } else {
        popup = popup.role("listbox").label("texlab 자동완성");
        let first = s.language.selected.saturating_sub(7);
        for (index, item) in s
            .language
            .completions
            .iter()
            .enumerate()
            .skip(first)
            .take(8)
        {
            popup = popup.child(
                row()
                    .key(format!("completion-{index}"))
                    .h(26.0)
                    .px(10.0)
                    .gap(10.0)
                    .center_y()
                    .fill(c(if index == s.language.selected {
                        "#34484c"
                    } else {
                        "#1e1e1e"
                    }))
                    .hover(c("#292929"))
                    .role("option")
                    .label(&item.label)
                    .on_press_tx(move |s: &mut State, tx| s.accept_completion(index, tx))
                    .child(mono(&item.label, 12.0, "#e6e6e6").grow(1.0).truncate())
                    .child(label(&item.detail, 10.0, "#8c8c8c").max_w(140.0).truncate()),
            );
        }
        popup = popup.child(
            label("↑↓ 선택 · Tab/Enter 적용 · Esc 닫기", 10.0, "#8c8c8c")
                .px(10.0)
                .py(6.0),
        );
    }
    popup
}

fn statusbar(s: &State) -> El {
    let (line, column) = s.cursor_position();
    row()
        .key("statusbar")
        .h(28.0)
        .w_full()
        .px(16.0)
        .gap(16.0)
        .center_y()
        .fill(c("#080808"))
        .border_sides([1.0, 0.0, 0.0, 0.0], c("#272727"))
        .child(glyph(if s.compiling { "clock" } else { "check" }, 16.0).color(c("#8c8c8c")))
        .child(
            label(&s.message, 12.0, "#aaaaaa")
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(
            label(&s.language.client.status, 12.0, "#8c8c8c")
                .help("Ctrl Space 자동완성 · F12 정의로 이동 · F1 설명"),
        )
        .child(mono(format!("줄 {line}, 열 {column}"), 12.0, "#8c8c8c"))
        .child(label("UTF-8", 12.0, "#8c8c8c"))
        .child(label(s.engine.label(), 12.0, "#8c8c8c"))
        .child(label(
            if s.active_file().path.ends_with(".bib") {
                "BibTeX"
            } else {
                "LaTeX"
            },
            12.0,
            "#8c8c8c",
        ))
}

fn dialog(s: &State, height: f32) -> El {
    let kind = s.dialog.as_ref().expect("dialog is open");
    let title = match kind {
        DialogKind::Search => "프로젝트 검색",
        DialogKind::NewFile => "새 파일",
        DialogKind::Settings => "편집기 설정",
    };
    let mut body = col()
        .gap(16.0)
        .padding(20.0)
        .min_h(0.0)
        .max_h((height - 180.0).clamp(180.0, 600.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    match kind {
        DialogKind::Search => {
            body = body.child(
                text_input(&s.search, TextStyle::new(14.0, 400.0))
                    .key("search-input")
                    .single_line()
                    .w_full()
                    .h(40.0)
                    .padding(10.0)
                    .color(c("#e6e6e6"))
                    .fill(c("#111111"))
                    .border(1.0, c("#414141"))
                    .radius(4.0)
                    .placeholder("파일 이름 또는 문서 내용 검색", c("#8c8c8c"))
                    .on_edit(|s: &mut State| &mut s.search)
                    .on_submit(|s: &mut State, tx| {
                        if let Some(item) = s.search_results().first() {
                            let (file, line) = (item.file, item.line.unwrap_or(1));
                            s.dialog = None;
                            s.jump_to(file, line, tx);
                        }
                    }),
            );
            let results = s.search_results();
            body = body.child(label(
                format!("{}개의 결과", results.len()),
                11.0,
                "#8c8c8c",
            ));
            let mut list = col().h(300.0).min_h(0.0).gap(4.0).scroll(ScrollSpec {
                stick_bottom: false,
            });
            for (i, result) in results.iter().take(100).enumerate() {
                let file = result.file;
                let line = result.line.unwrap_or(1);
                list = list.child(
                    col()
                        .key(format!("search-result-{i}"))
                        .padding(10.0)
                        .gap(4.0)
                        .radius(4.0)
                        .hover(c("#292929"))
                        .role("button")
                        .label(&result.label)
                        .focusable(true)
                        .on_press_tx(move |s: &mut State, tx| {
                            s.dialog = None;
                            s.jump_to(file, line, tx);
                        })
                        .child(mono(&result.label, 12.0, "#e6e6e6"))
                        .child(label(&result.excerpt, 11.0, "#8c8c8c")),
                );
            }
            body = body.child(list).child(label(
                "Enter로 첫 결과 열기   ·   Esc로 닫기",
                11.0,
                "#8c8c8c",
            ));
        }
        DialogKind::NewFile => {
            body = body
                .child(label("파일 이름", 12.0, "#aaaaaa"))
                .child(
                    text_input(&s.new_name, TextStyle::new(14.0, 400.0))
                        .key("new-name-input")
                        .single_line()
                        .w_full()
                        .h(40.0)
                        .padding(10.0)
                        .color(c("#e6e6e6"))
                        .fill(c("#111111"))
                        .border(1.0, c("#414141"))
                        .radius(4.0)
                        .placeholder("appendix.tex", c("#8c8c8c"))
                        .on_edit(|s: &mut State| &mut s.new_name)
                        .on_submit(|s: &mut State, tx| {
                            if s.add_file() {
                                s.dialog = None;
                                tx.focus("source-editor", false);
                            }
                        }),
                )
                .child(label(
                    ".tex, .bib, .sty 파일을 만들 수 있습니다.",
                    11.0,
                    "#8c8c8c",
                ))
                .child(label(&s.message, 11.0, "#e0ba80"))
                .child(
                    row()
                        .justify_end()
                        .gap(8.0)
                        .child(button("cancel-new", "취소", |s| s.dialog = None))
                        .child(button("create-file", "파일 만들기", |_| {}).on_press_tx(
                            |s: &mut State, tx| {
                                if s.add_file() {
                                    s.dialog = None;
                                    tx.focus("source-editor", false);
                                }
                            },
                        )),
                );
        }
        DialogKind::Settings => {
            body = settings::view(s, height);
        }
    }
    col()
        .key("modal-backdrop")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK.alpha(0.65))
        .items_center()
        .pt(64.0)
        .child(
            col()
                .key("dialog")
                .w(if *kind == DialogKind::Settings {
                    640.0
                } else {
                    560.0
                })
                .max_h((height - 112.0).max(320.0))
                .min_h(0.0)
                .radius(8.0)
                .fill(c("#171717"))
                .border(1.0, c("#414141"))
                .modal(true, |s: &mut State| s.dialog = None)
                .role("dialog")
                .label(title)
                .child(
                    row()
                        .h(56.0)
                        .px(24.0)
                        .center_y()
                        .justify_between()
                        .border_sides([0.0, 0.0, 1.0, 0.0], c("#272727"))
                        .child(label(title, 16.0, "#e6e6e6"))
                        .child(icon_button(
                            "close-dialog",
                            "x",
                            "대화상자 닫기",
                            |s| s.dialog = None,
                        )),
                )
                .child(body)
                .child(if *kind == DialogKind::Settings {
                    settings::footer()
                } else {
                    el()
                }),
        )
}
