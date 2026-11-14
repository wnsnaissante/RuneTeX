use super::*;
use crate::font_picker::FontTarget;

fn section(key: &str, title: &str, content: El) -> El {
    col()
        .key(key)
        .w_full()
        .gap(16.0)
        .child(label(title, 14.0, "#e6e6e6").font(TextStyle::new(14.0, 500.0)))
        .child(content)
}

fn switch(key: &str, title: &str, enabled: bool, action: fn(&mut State)) -> El {
    row()
        .key(key)
        .size(48.0, 32.0)
        .center()
        .radius(4.0)
        .role("switch")
        .label(title)
        .checked(enabled)
        .focusable(true)
        .focus_ring(1.5, 2.0, c("#78c4ce"))
        .help(if enabled { "켜짐" } else { "꺼짐" })
        .hover(c("#252525"))
        .on_press(action)
        .child(
            el().size(36.0, 20.0)
                .radius(10.0)
                .fill(c(if enabled { "#78c4ce" } else { "#414141" }))
                .child(
                    el().size(16.0, 16.0)
                        .radius(8.0)
                        .fill(c(if enabled { "#111111" } else { "#e6e6e6" }))
                        .absolute(if enabled { 18.0 } else { 2.0 }, 2.0),
                ),
        )
}

fn toggle_row(
    key: &str,
    title: &str,
    description: &str,
    enabled: bool,
    action: fn(&mut State),
) -> El {
    row()
        .w_full()
        .min_h(48.0)
        .center_y()
        .gap(16.0)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(label(title, 13.0, "#e6e6e6"))
                .child(label(description, 12.0, "#8c8c8c")),
        )
        .child(switch(key, title, enabled, action))
}

pub(super) fn view(s: &State, height: f32) -> El {
    let list_height = (height - 640.0).clamp(80.0, 224.0);
    let editing = col()
        .gap(16.0)
        .child(font_picker::view(s, FontTarget::Ui, list_height, false))
        .child(font_picker::view(
            s,
            FontTarget::Editor,
            list_height,
            height < 640.0,
        ))
        .child(
            row()
                .w_full()
                .h(36.0)
                .center_y()
                .gap(8.0)
                .child(label("에디터 글자 크기", 13.0, "#e6e6e6").grow(1.0))
                .child(
                    row()
                        .gap(4.0)
                        .center_y()
                        .fill(c("#111111"))
                        .border(1.0, c("#343434"))
                        .radius(4.0)
                        .child(icon_button(
                            "font-minus",
                            "minus",
                            "글자 크기 줄이기 · Ctrl -",
                            |s| s.change_editor_font_size(-1.0),
                        ))
                        .child(
                            mono(format!("{:.0}px", s.font_size), 12.0, "#e6e6e6")
                                .w(40.0)
                                .align(rune::Align::Center),
                        )
                        .child(icon_button(
                            "font-plus",
                            "plus",
                            "글자 크기 늘리기 · Ctrl +",
                            |s| s.change_editor_font_size(1.0),
                        )),
                ),
        );

    let mut engines = row()
        .gap(4.0)
        .padding(4.0)
        .fill(c("#111111"))
        .border(1.0, c("#343434"))
        .radius(4.0)
        .role("group")
        .label("LaTeX 엔진");
    for engine in [Engine::Pdf, Engine::Xe, Engine::Lua] {
        engines = engines.child(
            row()
                .key(format!("engine-{}", engine.executable()))
                .h(32.0)
                .px(12.0)
                .center()
                .radius(4.0)
                .fill(c(if s.engine == engine {
                    "#303030"
                } else {
                    "#111111"
                }))
                .hover(c("#292929"))
                .role("button")
                .label(engine.label())
                .checked(s.engine == engine)
                .focusable(true)
                .focus_ring(1.5, 2.0, c("#78c4ce"))
                .on_press(move |s: &mut State| s.engine = engine)
                .child(label(
                    engine.label(),
                    13.0,
                    if s.engine == engine {
                        "#e6e6e6"
                    } else {
                        "#8c8c8c"
                    },
                )),
        );
    }
    let compiling = col()
        .gap(16.0)
        .child(toggle_row(
            "auto-compile",
            "자동 컴파일",
            "입력을 멈추면 PDF 갱신",
            s.auto_compile,
            |s| s.auto_compile = !s.auto_compile,
        ))
        .child(toggle_row(
            "auto-install-packages",
            "자동 패키지 설치",
            "MiKTeX의 누락된 패키지 다운로드",
            s.auto_install_packages,
            State::toggle_package_installer,
        ))
        .child(
            col()
                .gap(8.0)
                .child(label("LaTeX 엔진", 13.0, "#e6e6e6"))
                .child(engines)
                .child(label("한글 문서에는 XeLaTeX와 kotex 사용", 12.0, "#8c8c8c")),
        );

    let language = row()
        .w_full()
        .center_y()
        .gap(16.0)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(label("texlab", 13.0, "#e6e6e6"))
                .child(label(&s.language.client.status, 12.0, "#8c8c8c").truncate()),
        )
        .child(button("restart-lsp", "재연결", State::restart_lsp));

    let mut shortcuts = col().gap(4.0);
    for (title, chord) in [
        ("에디터 글자 크기", "Ctrl + / Ctrl -"),
        ("자동완성", "Ctrl Space"),
        ("정의로 이동", "F12"),
        ("명령 설명", "F1"),
        ("프로젝트 열기", "Ctrl O"),
        ("프로젝트 검색", "Ctrl K"),
        ("컴파일", "Ctrl Enter"),
        ("모든 파일 저장", "Ctrl S"),
        ("탐색기 열기·닫기", "Ctrl B"),
    ] {
        shortcuts = shortcuts.child(
            row()
                .w_full()
                .h(32.0)
                .center_y()
                .justify_between()
                .child(label(title, 13.0, "#aaaaaa"))
                .child(keycap(chord)),
        );
    }

    col()
        .key("settings-scroll")
        .w_full()
        .min_h(0.0)
        .max_h((height - 240.0).clamp(200.0, 600.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(24.0)
        .gap(24.0)
        .child(section("settings-editing", "글꼴과 편집", editing).paint_order(10))
        .child(el().w_full().h(1.0).fill(c("#272727")))
        .child(section("settings-compiling", "컴파일", compiling))
        .child(el().w_full().h(1.0).fill(c("#272727")))
        .child(section("settings-language", "언어 서버", language))
        .child(el().w_full().h(1.0).fill(c("#272727")))
        .child(section("settings-shortcuts", "단축키", shortcuts))
}

pub(super) fn footer() -> El {
    row()
        .key("settings-footer")
        .w_full()
        .h(64.0)
        .px(24.0)
        .gap(16.0)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], c("#272727"))
        .child(label("글꼴 변경은 각 적용 버튼으로 반영", 12.0, "#8c8c8c").grow(1.0))
        .child(primary_button("close-settings", "완료", |s| {
            s.dialog = None
        }))
}
