use rune::{El, Icon};

pub fn glyph(name: &str, size: f32) -> El {
    static ICONS: &[Icon] = &[
        Icon {
            name: "download",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12m-4-4 4 4 4-4M4 16v4h16v-4"/></g>"#,
        },
        Icon {
            name: "panel-left",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/></g>"#,
        },
        Icon {
            name: "search",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4.5 4.5"/></g>"#,
        },
        Icon {
            name: "sliders",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M4 7h8m6 0h2M4 17h2m6 0h8"/><circle cx="15" cy="7" r="3"/><circle cx="9" cy="17" r="3"/></g>"#,
        },
        Icon {
            name: "play",
            view: 24.0,
            body: r#"<path d="m9 5 11 7-11 7V5Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/>"#,
        },
        Icon {
            name: "folder",
            view: 24.0,
            body: r#"<path d="M3 7V5.5A1.5 1.5 0 0 1 4.5 4h5L12 7h7.5A1.5 1.5 0 0 1 21 8.5v10a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5V7Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/>"#,
        },
        Icon {
            name: "file",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M13 3H6a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V9l-6-6Z"/><path d="M13 3v6h6M9 13h6m-6 4h4"/></g>"#,
        },
        Icon {
            name: "book",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3h13v18H6a3 3 0 0 1 0-6h13M3 18V6a3 3 0 0 1 3-3m2 0v12"/><path d="M11 7h5m-5 3h4"/></g>"#,
        },
        Icon {
            name: "plus",
            view: 24.0,
            body: r#"<path d="M12 5v14M5 12h14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>"#,
        },
        Icon {
            name: "minus",
            view: 24.0,
            body: r#"<path d="M5 12h14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>"#,
        },
        Icon {
            name: "x",
            view: 24.0,
            body: r#"<path d="m6 6 12 12M18 6 6 18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>"#,
        },
        Icon {
            name: "chevron-down",
            view: 24.0,
            body: r#"<path d="m6 9 6 6 6-6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>"#,
        },
        Icon {
            name: "chevron-up",
            view: 24.0,
            body: r#"<path d="m6 15 6-6 6 6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>"#,
        },
        Icon {
            name: "chevron-right",
            view: 24.0,
            body: r#"<path d="m9 6 6 6-6 6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>"#,
        },
        Icon {
            name: "check",
            view: 24.0,
            body: r#"<path d="m5 12 4 4L19 6" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>"#,
        },
        Icon {
            name: "circle-check",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="m8 12 3 3 5-6"/></g>"#,
        },
        Icon {
            name: "clock",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></g>"#,
        },
        Icon {
            name: "code",
            view: 24.0,
            body: r#"<path d="m7 7-5 5 5 5m10-10 5 5-5 5m-6 3 3-16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>"#,
        },
        Icon {
            name: "split",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M12 4v16"/></g>"#,
        },
        Icon {
            name: "eye",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/></g>"#,
        },
        Icon {
            name: "maximize",
            view: 24.0,
            body: r#"<path d="M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>"#,
        },
        Icon {
            name: "branch",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="5" r="2"/><path d="M6 7v10m12-10v3a5 5 0 0 1-5 5H6"/></g>"#,
        },
        Icon {
            name: "lock",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V6a4 4 0 0 1 8 0v4m-4 5v2"/></g>"#,
        },
        Icon {
            name: "arrow-left",
            view: 24.0,
            body: r#"<path d="M20 12H4m6-6-6 6 6 6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>"#,
        },
        Icon {
            name: "alert",
            view: 24.0,
            body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="m12 3 10 18H2L12 3Z"/><path d="M12 9v5m0 3v.1"/></g>"#,
        },
    ];
    let icon = ICONS
        .iter()
        .find(|icon| icon.name == name)
        .unwrap_or(&ICONS[0]);
    rune::icon(icon, size)
}
