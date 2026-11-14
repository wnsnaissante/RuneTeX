use super::{
    icons::glyph,
    state::{SetupState, Stage},
};
use rune::{
    Color, Cx, El, Family, Key, KeyEvent, ScrollSpec, TextStyle, Tx, col, el, row, text, text_input,
};

const BG: &str = "#171717";
const INPUT: &str = "#111111";
const LINE: &str = "#2b2b2b";
const TEXT: &str = "#e6e6e6";
const SECONDARY: &str = "#aaaaaa";
const MUTED: &str = "#8c8c8c";
const ACCENT: &str = "#78c4ce";

fn c(value: &str) -> Color {
    rune::color::hex(value).expect("valid color")
}
fn label(value: impl Into<String>, size: f32, color: &str) -> El {
    text(value.into())
        .font(TextStyle::new(size, 400.0))
        .color(c(color))
}
fn heading(value: &str, size: f32) -> El {
    label(value, size, TEXT).font(TextStyle::new(size, 500.0))
}
fn mono(value: impl Into<String>, size: f32, color: &str) -> El {
    label(value, size, color).font(TextStyle::new(size, 400.0).family(Family::Mono))
}
fn spacer() -> El {
    el().grow(1.0).min_w(0.0).min_h(0.0)
}
fn button(key: &str, title: &str, primary: bool) -> El {
    row()
        .key(key)
        .h(36.0)
        .px(16.0)
        .center()
        .radius(4.0)
        .fill(c(if primary { TEXT } else { BG }))
        .border(1.0, c(if primary { TEXT } else { "#343434" }))
        .hover(c(if primary { "#ffffff" } else { "#252525" }))
        .role("button")
        .label(title)
        .focusable(true)
        .focus_ring(1.5, 2.0, c(ACCENT))
        .child(label(title, 13.0, if primary { INPUT } else { TEXT }))
}

pub fn on_key(s: &mut SetupState, key: &KeyEvent, tx: &mut Tx) -> bool {
    if key.key == Key::Escape {
        s.cancel_or_close(tx);
        return true;
    }

    false
}

pub fn view(s: &SetupState, cx: &mut Cx<SetupState>) -> El {
    let width = cx
        .layout_size("setup-window")
        .map_or(1120.0, |size| size[0]);
    let summary_width = ((width - 64.0) * 0.34).clamp(296.0, 380.0);
    let content = match s.stage {
        Stage::Configure => configure(s, summary_width),
        Stage::Progress => progress(s, summary_width),
        Stage::Complete => complete(s, summary_width),
    };
    col()
        .key("setup-window")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(c(BG))
        .child(header(s))
        .child(
            col()
                .key("setup-body")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .px(32.0)
                .py(24.0)
                .child(content),
        )
        .child(footer(s))
}

fn header(s: &SetupState) -> El {
    let mut steps = row().gap(16.0).center_y();
    for (index, stage, title) in [
        (0, Stage::Configure, "설정"),
        (1, Stage::Progress, if s.removing { "제거" } else { "설치" }),
        (2, Stage::Complete, "완료"),
    ] {
        let active = s.stage.index() == index;
        steps = steps.child(
            row()
                .key(format!("setup-step-{index}"))
                .h(32.0)
                .px(4.0)
                .center_y()
                .border_sides([0.0, 0.0, if active { 2.0 } else { 0.0 }, 0.0], c(ACCENT))
                .hover(c(if s.preview { "#222222" } else { BG }))
                .role(if s.preview { "button" } else { "text" })
                .label(if s.preview {
                    format!("{title} 화면 미리보기")
                } else {
                    title.to_string()
                })
                .checked(active)
                .focusable(s.preview)
                .focus_ring(1.5, 2.0, c(ACCENT))
                .on_press(move |s: &mut SetupState| s.select_stage(stage))
                .child(label(title, 13.0, if active { TEXT } else { MUTED })),
        );
    }
    row()
        .key("setup-header")
        .w_full()
        .h(56.0)
        .px(32.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], c(LINE))
        .child(heading(
            if s.removing {
                "RuneTeX 제거"
            } else {
                "RuneTeX 설치"
            },
            16.0,
        ))
        .child(label(
            format!("{} · Windows x64", env!("CARGO_PKG_VERSION")),
            12.0,
            MUTED,
        ))
        .child(spacer())
        .child(steps)
}

fn title(value: &str, description: &str) -> El {
    col()
        .gap(8.0)
        .child(label(value, 24.0, TEXT))
        .child(label(description, 14.0, SECONDARY))
}

fn checkbox(key: &str, title: &str, checked: bool, action: fn(&mut SetupState)) -> El {
    row()
        .key(key)
        .h(32.0)
        .gap(12.0)
        .center_y()
        .role("checkbox")
        .label(title)
        .checked(checked)
        .focusable(true)
        .focus_ring(1.5, 2.0, c(ACCENT))
        .on_press(action)
        .child(
            row()
                .size(16.0, 16.0)
                .center()
                .radius(3.0)
                .fill(c(if checked { ACCENT } else { INPUT }))
                .border(1.0, c(if checked { ACCENT } else { "#555555" }))
                .child(if checked {
                    glyph("check", 12.0).color(c(INPUT))
                } else {
                    el()
                }),
        )
        .child(label(title, 14.0, TEXT))
}

fn summary(width: f32, completed: bool) -> El {
    let mut content = col()
        .key("setup-components")
        .w(width)
        .min_w(0.0)
        .pl(24.0)
        .gap(8.0)
        .border_sides([0.0, 0.0, 0.0, 1.0], c(LINE))
        .child(heading(
            if completed {
                "설치된 항목"
            } else {
                "설치 항목"
            },
            14.0,
        ));
    for (name, description) in [
        ("RuneTeX", "LaTeX 편집기"),
        ("texlab", "자동완성 · 오류 진단"),
        ("Poppler", "PDF 미리보기"),
    ] {
        content = content.child(
            row()
                .w_full()
                .h(36.0)
                .gap(12.0)
                .center_y()
                .child(label(name, 14.0, TEXT).w(88.0))
                .child(label(description, 13.0, SECONDARY).grow(1.0).min_w(0.0)),
        );
    }
    content.child(
        label("MiKTeX · TeX Live 별도 설치", 13.0, MUTED)
            .w((width - 24.0).max(0.0))
            .wrap()
            .mt(8.0),
    )
}

fn configure(s: &SetupState, summary_width: f32) -> El {
    let path = col()
        .gap(8.0)
        .child(heading(
            if s.removing {
                "제거할 위치"
            } else {
                "설치 위치"
            },
            14.0,
        ))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text_input(&s.destination, TextStyle::new(14.0, 400.0))
                        .key("setup-destination")
                        .label("설치 위치")
                        .invalid(s.error.is_some())
                        .single_line()
                        .disabled(s.removing || s.running)
                        .h(40.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .px(12.0)
                        .py(10.0)
                        .fill(c(INPUT))
                        .border(
                            1.0,
                            c(if s.error.is_some() {
                                "#e0ba80"
                            } else {
                                "#343434"
                            }),
                        )
                        .radius(4.0)
                        .color(c(TEXT))
                        .caret(c(ACCENT), c("#34484c"))
                        .on_edit(|s: &mut SetupState| &mut s.destination)
                        .on_input_change(|s: &mut SetupState, _| s.error = None),
                )
                .child(
                    button("setup-browse", "찾아보기", false)
                        .h(40.0)
                        .disabled(s.removing || s.running)
                        .on_press_tx(SetupState::browse),
                ),
        )
        .child(if let Some(error) = &s.error {
            label(error, 13.0, "#e0ba80")
                .w_full()
                .wrap_anywhere()
                .role("alert")
        } else {
            label("문서와 개인 설정은 이 폴더 밖에 보관됩니다", 13.0, MUTED)
        });
    let mut options = col()
        .key("setup-options")
        .grow(1.0)
        .min_w(0.0)
        .gap(24.0)
        .child(path);
    if !s.removing {
        options = options.child(
            col().gap(8.0).child(heading("바로가기", 14.0)).child(
                row()
                    .gap(24.0)
                    .center_y()
                    .child(checkbox(
                        "setup-start-menu",
                        "시작 메뉴에 추가",
                        s.start_menu,
                        |s| s.start_menu = !s.start_menu,
                    ))
                    .child(checkbox(
                        "setup-desktop",
                        "바탕화면에 추가",
                        s.desktop_shortcut,
                        |s| s.desktop_shortcut = !s.desktop_shortcut,
                    )),
            ),
        );
    }

    col()
        .w_full()
        .gap(24.0)
        .child(title(
            if s.removing {
                "제거 확인"
            } else {
                "설치 설정"
            },
            if s.removing {
                "프로그램과 바로가기를 제거합니다. 문서와 개인 설정은 유지됩니다"
            } else {
                "현재 사용자 계정에 설치합니다"
            },
        ))
        .child(
            row()
                .key("setup-columns")
                .w_full()
                .min_w(0.0)
                .gap(32.0)
                .child(options)
                .child(if s.removing {
                    col()
                        .w(summary_width)
                        .pl(24.0)
                        .border_sides([0.0, 0.0, 0.0, 1.0], c(LINE))
                        .gap(8.0)
                        .child(heading("제거 항목", 14.0))
                        .child(label("RuneTeX · texlab · Poppler", 14.0, SECONDARY))
                        .child(
                            label("설치된 프로그램과 만든 바로가기를 제거합니다", 13.0, MUTED)
                                .w_full()
                                .wrap_anywhere(),
                        )
                } else {
                    summary(summary_width, false)
                }),
        )
}

fn progress(s: &SetupState, summary_width: f32) -> El {
    let meter = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(16.0)
        .child(
            row()
                .w_full()
                .center_y()
                .justify_between()
                .child(heading(
                    if s.removing {
                        "프로그램 제거"
                    } else {
                        "앱과 구성 요소 복사"
                    },
                    14.0,
                ))
                .child(mono(
                    format!("{:.0}%", s.percent.clamp(0.0, 100.0)),
                    28.0,
                    TEXT,
                )),
        )
        .child(
            row()
                .key("setup-progress")
                .w_full()
                .h(4.0)
                .fill(c("#303030"))
                .role("progressbar")
                .label(if s.removing {
                    "제거 진행률"
                } else {
                    "설치 진행률"
                })
                .range_value(s.percent.clamp(0.0, 100.0), 0.0, 100.0)
                .child(
                    el().w_frac((s.percent.clamp(0.0, 100.0) / 100.0) as f32)
                        .h(4.0)
                        .fill(c(ACCENT)),
                ),
        )
        .child(mono(&s.current_file, 12.0, MUTED).w_full().truncate());
    let mut tasks = col()
        .w(summary_width)
        .pl(24.0)
        .gap(8.0)
        .border_sides([0.0, 0.0, 0.0, 1.0], c(LINE))
        .child(heading(
            if s.removing {
                "제거 상태"
            } else {
                "설치 상태"
            },
            14.0,
        ));
    for (index, name) in if s.removing {
        [
            (0, "제거 준비"),
            (1, "프로그램 파일 제거"),
            (2, "바로가기와 등록 정리"),
        ]
    } else {
        [
            (0, "설치 폴더 준비"),
            (1, "구성 요소 복사"),
            (2, "바로가기 만들기"),
        ]
    } {
        let active = s.phase == index;
        let done = s.phase > index;
        let icon = if done {
            "check"
        } else if active {
            "clock"
        } else {
            "file"
        };
        let status = if done {
            "완료"
        } else if active {
            "진행 중"
        } else {
            "대기"
        };
        tasks = tasks.child(
            row()
                .w_full()
                .h(40.0)
                .gap(12.0)
                .center_y()
                .child(glyph(icon, 16.0).color(c(if active { ACCENT } else { MUTED })))
                .child(label(name, 13.0, TEXT).grow(1.0))
                .child(label(status, 12.0, if active { ACCENT } else { MUTED })),
        );
    }
    col()
        .w_full()
        .gap(24.0)
        .child(title(
            if s.removing {
                "제거 중"
            } else {
                "설치 중"
            },
            if s.cancel_requested {
                "설치를 취소하고 변경을 되돌리고 있습니다"
            } else if s.removing {
                "문서와 개인 설정을 보존하며 프로그램을 제거합니다"
            } else {
                "파일을 복사하고 바로가기를 준비합니다"
            },
        ))
        .child(
            row()
                .key("setup-columns")
                .w_full()
                .gap(32.0)
                .child(meter)
                .child(tasks),
        )
}

fn complete(s: &SetupState, summary_width: f32) -> El {
    let mut shortcuts = Vec::new();
    if s.start_menu {
        shortcuts.push("시작 메뉴");
    }
    if s.desktop_shortcut {
        shortcuts.push("바탕화면");
    }
    let description = if shortcuts.is_empty() {
        "만들지 않음".into()
    } else {
        shortcuts.join(" · ")
    };
    let mut result = col().grow(1.0).min_w(0.0).gap(24.0).child(
        col()
            .gap(8.0)
            .child(heading(
                if s.removing {
                    "제거한 위치"
                } else {
                    "설치 위치"
                },
                14.0,
            ))
            .child(
                label(&s.destination.text, 14.0, SECONDARY)
                    .w_full()
                    .wrap_anywhere(),
            ),
    );
    if !s.removing {
        result = result
            .child(col().gap(8.0).child(heading("바로가기", 14.0)).child(label(
                description,
                14.0,
                SECONDARY,
            )))
            .child(label(
                "TeX 엔진은 앱 설정에서 확인할 수 있습니다",
                13.0,
                MUTED,
            ));
    }
    col()
        .w_full()
        .gap(24.0)
        .child(title(
            if s.removing {
                "제거 완료"
            } else {
                "설치 완료"
            },
            if s.removing {
                "프로그램을 제거했습니다. 문서와 개인 설정은 유지됩니다"
            } else {
                "RuneTeX를 사용할 수 있습니다"
            },
        ))
        .child(
            row()
                .key("setup-columns")
                .w_full()
                .gap(32.0)
                .child(result)
                .child(if s.removing {
                    col().w(summary_width).pl(24.0).child(label(
                        "개인 문서와 설정은 삭제하지 않았습니다",
                        14.0,
                        SECONDARY,
                    ))
                } else {
                    summary(summary_width, true)
                }),
        )
}

fn footer(s: &SetupState) -> El {
    let primary = match s.stage {
        Stage::Configure => {
            if s.removing {
                "제거"
            } else {
                "설치"
            }
        }
        Stage::Progress => {
            if s.preview {
                "완료 화면 보기"
            } else {
                if s.removing {
                    "제거 중"
                } else {
                    "설치 중"
                }
            }
        }
        Stage::Complete => "마침",
    };
    let mut actions = row().gap(8.0).center_y();
    if s.stage != Stage::Complete {
        actions = actions.child(
            button("setup-cancel", "취소", false)
                .border(0.0, Color::TRANSPARENT)
                .disabled(s.cancel_requested || (s.removing && s.running))
                .on_press_tx(SetupState::cancel_or_close),
        );
    }
    actions = actions.child(
        button("setup-primary", primary, true)
            .disabled((s.running && !s.preview) || (!s.payload_ready && !s.removing && !s.preview))
            .on_press_tx(SetupState::advance),
    );
    row()
        .key("setup-footer")
        .w_full()
        .h(64.0)
        .px(32.0)
        .gap(16.0)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], c(LINE))
        .child(
            label(
                if s.preview {
                    "디자인 미리보기 · 실제 파일은 설치하지 않습니다"
                } else if s.cancel_requested {
                    &s.status
                } else if s.removing {
                    "문서와 개인 설정은 유지됩니다"
                } else {
                    "현재 사용자 계정에 설치"
                },
                12.0,
                MUTED,
            )
            .grow(1.0)
            .min_w(0.0),
        )
        .child(actions)
}
