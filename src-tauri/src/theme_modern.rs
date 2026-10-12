//! Split light/dark skins (observed in 0.9.2.7). Only the dark channel is themed.
//! Sources are read from the user's installation, never bundled with the helper.
use crate::{fonts::FontChoice, theme, transaction::Files};
use std::collections::BTreeSet;

const CAPS: &[&str] = &["lang_caps_normal.svg", "lang_caps_hover.svg"];

pub fn paths() -> BTreeSet<String> {
    let mut paths: BTreeSet<_> = [
        "window.xml",
        "color/color_dark.xml",
        "dark/composition/page_open.svg",
        "dark/status_wnd/logo.png",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for i in 1..=7 {
        paths.insert(format!("dark/composition/bk_image_{i}.svg"));
    }
    for name in theme::STATUS_WND_FILES.iter().chain(CAPS) {
        paths.insert(format!("dark/status_wnd/{name}"));
    }
    paths
}

/// Match only the known opaque face, not the white border backing or filters.
pub fn background(src: &str, old: &str, color: &str, opacity: u8) -> Result<String, String> {
    let needle = format!(r#"fill="{old}""#);
    if src.matches(&needle).count() != 1 {
        return Err("深色背景 SVG 结构变化：找不到唯一的背景填充".into());
    }
    let replacement = format!(
        r#"fill="{}" fill-opacity="{}""#,
        theme::normalize_hex(color)?,
        theme::glass_opacity_attr(opacity)
    );
    // A new upstream opacity would need a deliberate adapter, not duplicate attrs.
    let start = src.find(&needle).unwrap();
    let tag_start = src[..start].rfind('<').ok_or("无效背景 SVG")?;
    let tag_end = start + src[start..].find('>').ok_or("无效背景 SVG")?;
    if src[tag_start..tag_end].contains("fill-opacity=") {
        return Err("深色背景填充透明度结构已变化".into());
    }
    let out = src.replacen(&needle, &replacement, 1);
    theme::validate_xml(&out)?;
    Ok(out)
}

pub fn icon(src: &str, colors: &theme::ThemeColors) -> Result<String, String> {
    let normal = r#"fill="white" fill-opacity="0.85""#;
    let accent = r##"fill="#5B8CFF""##;
    if !src.contains(normal) && !src.contains(accent) {
        return Err("深色工具栏图标结构已变化".into());
    }
    let out = src
        .replace(
            normal,
            &format!(
                r#"fill="{}" fill-opacity="0.85""#,
                theme::normalize_hex(&colors.foreground)?
            ),
        )
        .replace(
            accent,
            &format!(r#"fill="{}""#, theme::normalize_hex(&colors.accent)?),
        );
    // White / 6% hover backing, path geometry and other opacities stay untouched.
    theme::validate_xml(&out)?;
    Ok(out)
}

fn check_references(xml: &str) -> Result<(), String> {
    // Require the semantic slots that this adapter edits, not just file names.
    for (name, attr, expected) in [
        ("index_style", "textcolor", "candidate_index_text"),
        (
            "index_style",
            "selectedtextcolor",
            "candidate_selected_text",
        ),
        ("cand_style", "textcolor", "candidate_text"),
        ("cand_style", "selectedtextcolor", "candidate_selected_text"),
        ("more_cand_style", "textcolor", "candidate_text"),
        (
            "more_cand_style",
            "selectedtextcolor",
            "candidate_selected_text",
        ),
        ("cand_container", "selectedbkcolor", "primary"),
        ("more_cand_container", "selectedbkcolor", "primary"),
        ("scroll_style", "thumbnormalcolor", "scrollbar_thumb"),
        ("scroll_style", "thumbhotcolor", "scrollbar_thumb_hover"),
    ] {
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut matches = 0;
        loop {
            match reader.read_event().map_err(|e| e.to_string())? {
                quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e)
                    if e.name().as_ref() == "Class" =>
                {
                    let attrs = e
                        .attributes()
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())?;
                    if attrs
                        .iter()
                        .any(|a| a.key.as_ref() == "name" && a.value.as_ref() == name)
                    {
                        matches += 1;
                        if !attrs
                            .iter()
                            .any(|a| a.key.as_ref() == attr && a.value.as_ref() == expected)
                        {
                            return Err(format!("深色颜色引用变化：{name}.{attr}"));
                        }
                    }
                }
                quick_xml::events::Event::Eof => break,
                _ => (),
            }
        }
        if matches != 1 {
            return Err(format!("深色候选栏结构变化：{name}"));
        }
    }
    Ok(())
}

pub fn generate(
    originals: &Files,
    colors: &theme::ThemeColors,
    fonts: &FontChoice,
    opacity: u8,
    logo: Option<&[u8]>,
) -> Result<Files, String> {
    if originals.keys().ne(paths().iter()) {
        return Err("深色皮肤文件清单不完整".into());
    }
    let colors = colors.clone().normalize()?;
    let fonts = fonts.clone().normalize()?;
    let text = |path: &str| -> Result<&str, String> {
        std::str::from_utf8(originals.get(path).ok_or_else(|| format!("缺少 {path}"))?)
            .map_err(|e| e.to_string())
    };
    let mut out = originals.clone();
    check_references(text("window.xml")?)?;
    // Shared XML: only font faces change. Light colours/layout remain official.
    let window = theme::official_window_xml(text("window.xml")?, &fonts)?;
    theme::validate_xml(&window)?;
    out.insert("window.xml".into(), window.into_bytes());
    let mut dictionary = text("color/color_dark.xml")?.to_owned();
    for (name, color, alpha) in [
        (
            "primary",
            &colors.accent,
            theme::percent_to_alpha(theme::clamp_glass_opacity(opacity)),
        ),
        ("candidate_text", &colors.foreground, 0xf2),
        ("candidate_index_text", &colors.foreground, 0xb3),
        ("candidate_selected_text", &colors.emphasis, 0xff),
        ("candidate_row_highlight", &colors.foreground, 0x14),
        ("scrollbar_thumb", &colors.foreground, 0x4d),
        ("scrollbar_thumb_hover", &colors.foreground, 0x73),
    ] {
        dictionary = theme::set_tag_attrs(
            &dictionary,
            "Color",
            Some(name),
            &[("value", &theme::argb(color, alpha)?)],
        )?;
    }
    theme::validate_xml(&dictionary)?;
    out.insert("color/color_dark.xml".into(), dictionary.into_bytes());
    for i in 1..=7 {
        let path = format!("dark/composition/bk_image_{i}.svg");
        out.insert(
            path.clone(),
            background(text(&path)?, "#151515", &colors.background, opacity)?.into_bytes(),
        );
    }
    let path = "dark/composition/page_open.svg";
    let chevron = text(path)?;
    let needle = r#"fill="white" fill-opacity="0.35""#;
    if chevron.matches(needle).count() != 1 {
        return Err("深色展开箭头结构已变化".into());
    }
    let chevron = chevron.replacen(
        needle,
        &format!(r#"fill="{}" fill-opacity="0.72""#, colors.foreground),
        1,
    );
    theme::validate_xml(&chevron)?;
    out.insert(path.into(), chevron.into_bytes());
    for name in theme::STATUS_WND_FILES.iter().chain(CAPS) {
        let path = format!("dark/status_wnd/{name}");
        let raw = text(&path)?;
        let svg = match *name {
            "status_bar_bg.svg" => background(raw, "#262626", &colors.background, opacity)?,
            "status_divider.svg" => raw.to_owned(),
            _ => icon(raw, &colors)?,
        };
        out.insert(path, svg.into_bytes());
    }
    if let Some(logo) = logo {
        crate::avatar::validate_stored(logo)?;
        out.insert("dark/status_wnd/logo.png".into(), logo.to_vec());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Files {
        let mut files: Files = paths().into_iter().map(|p| (p, Vec::new())).collect();
        files.insert("window.xml".into(), br#"<Window><Font name="word" facename="old"/><Font name="number" facename="old"/><Class name="index_style" textcolor="candidate_index_text" selectedtextcolor="candidate_selected_text"/><Class name="cand_style" textcolor="candidate_text" selectedtextcolor="candidate_selected_text"/><Class name="more_cand_style" textcolor="candidate_text" selectedtextcolor="candidate_selected_text"/><Class name="cand_container" selectedbkcolor="primary"/><Class name="more_cand_container" selectedbkcolor="primary"/><Class name="scroll_style" thumbnormalcolor="scrollbar_thumb" thumbhotcolor="scrollbar_thumb_hover"/></Window>"#.to_vec());
        let colors: String = [
            "primary",
            "candidate_text",
            "candidate_index_text",
            "candidate_selected_text",
            "candidate_row_highlight",
            "scrollbar_thumb",
            "scrollbar_thumb_hover",
            "border_strong",
            "window_border",
            "menu_bg",
        ]
        .iter()
        .map(|n| format!(r##"<Color name="{n}" value="#FF010203"/>"##))
        .collect();
        files.insert(
            "color/color_dark.xml".into(),
            format!("<Colors>{colors}</Colors>").into_bytes(),
        );
        for (path, bytes) in &mut files {
            if path.ends_with(".svg") {
                *bytes = if path.contains("bk_image") {
                    br##"<svg><rect fill="white" fill-opacity="0.08"/><path fill="#151515"/><defs><filter id="shadow"/></defs></svg>"##.to_vec()
                } else if path.ends_with("status_bar_bg.svg") {
                    br##"<svg><rect fill="#262626"/><rect stroke="white" stroke-opacity="0.08" stroke-width="0.5"/></svg>"##.to_vec()
                } else if path.ends_with("page_open.svg") {
                    br#"<svg><path fill="white" fill-opacity="0.35"/></svg>"#.to_vec()
                } else {
                    br#"<svg><path fill="white" fill-opacity="0.85"/></svg>"#.to_vec()
                };
            }
        }
        files
    }
    #[test]
    fn complete_patch_uses_dark_dictionary_and_shared_fonts_only() {
        let original = fixture();
        for colors in [
            theme::dark_colors(),
            theme::light_colors(),
            theme::mix_colors(),
        ] {
            let out = generate(&original, &colors, &FontChoice::default(), 70, None).unwrap();
            assert!(out.keys().eq(original.keys()));
            let window = std::str::from_utf8(&out["window.xml"]).unwrap();
            assert!(window.contains(r#"textcolor="candidate_text""#));
            assert!(!window.contains(" bkcolor="));
            let dictionary = std::str::from_utf8(&out["color/color_dark.xml"]).unwrap();
            assert!(dictionary.contains(&format!(
                r#"name="primary" value="{}""#,
                theme::argb(&colors.accent, 179).unwrap()
            )));
            for name in ["border_strong", "window_border", "menu_bg"] {
                assert!(dictionary.contains(&format!(r##"name="{name}" value="#FF010203""##)));
            }
            assert_eq!(
                out["dark/status_wnd/status_divider.svg"],
                original["dark/status_wnd/status_divider.svg"]
            );
        }
        let mut changed = original;
        changed.insert(
            "window.xml".into(),
            std::str::from_utf8(&changed["window.xml"])
                .unwrap()
                .replace("candidate_text", "new_slot")
                .into_bytes(),
        );
        assert!(generate(
            &changed,
            &theme::dark_colors(),
            &FontChoice::default(),
            100,
            None
        )
        .is_err());
    }
    #[test]
    #[ignore = "read-only installed 0.9.2.7 sources; transactions run ONLY in a fresh temp directory"]
    fn installed_split_sources_read_only() {
        use crate::{safe_fs, skin, transaction::Store};
        let target = skin::DetectedIme {
            version: "v0.9.2.7".into(),
            skin: std::path::PathBuf::from(skin::IME_ROOT)
                .join("versions/v0.9.2.7/files/data/skin/default"),
        };
        assert_eq!(skin::layout(&target.skin), skin::Layout::SplitDark);
        skin::inspect_structure(&target.skin).unwrap();
        let live_store = skin::store_for(&target).unwrap();
        let before = live_store.live_hashes().unwrap();
        let original = live_store.originals().unwrap();
        let light_before =
            safe_fs::read(&target.skin.join("color/color_light.xml"), 65536).unwrap();
        let temp = crate::workdir::new_scratch().unwrap();
        for (path, bytes) in &original {
            safe_fs::atomic_write(&temp.join(path), bytes).unwrap();
        }
        let store = Store::new(
            temp.clone(),
            target.version.clone(),
            Store::hashes(&original),
        );
        safe_fs::atomic_write(
            &temp.join("light/composition/sentinel.svg"),
            b"untouched light",
        )
        .unwrap();
        for colors in [
            theme::dark_colors(),
            theme::light_colors(),
            theme::mix_colors(),
        ] {
            for opacity in [20, 70, 100] {
                let out = theme::generate_patch(
                    &original,
                    &colors,
                    &FontChoice::default(),
                    colors == theme::light_colors(),
                    opacity,
                    None,
                )
                .unwrap();
                for i in 1..=7 {
                    let path = format!("dark/composition/bk_image_{i}.svg");
                    let expected = std::str::from_utf8(&original[&path]).unwrap().replace(
                        r##"fill="#151515""##,
                        &format!(
                            r#"fill="{}" fill-opacity="{}""#,
                            colors.background,
                            theme::glass_opacity_attr(opacity)
                        ),
                    );
                    assert_eq!(out[&path], expected.as_bytes());
                }
                for (path, bytes) in &out {
                    if path.ends_with(".svg") || path.ends_with(".xml") {
                        theme::validate_xml(std::str::from_utf8(bytes).unwrap()).unwrap();
                    }
                }
                let preview = theme::generate_toolbar_preview(
                    &target.skin.join("dark/status_wnd"),
                    &colors,
                    opacity < 100,
                    opacity,
                    colors == theme::light_colors(),
                    None,
                )
                .unwrap();
                assert_eq!(preview.logo_size, 22);
                assert_eq!(
                    preview.bg.as_bytes(),
                    &out["dark/status_wnd/status_bar_bg.svg"]
                );
                assert_eq!(
                    preview.mic.as_bytes(),
                    &out["dark/status_wnd/mic_normal.svg"]
                );
                store
                    .apply(&out, &store.live_hashes().unwrap(), "test", "test-install")
                    .unwrap();
                assert_eq!(store.live_hashes().unwrap(), Store::hashes(&out));
                store
                    .apply(
                        &original,
                        &store.live_hashes().unwrap(),
                        "original",
                        "test-restore",
                    )
                    .unwrap();
                assert_eq!(store.live_hashes().unwrap(), Store::hashes(&original));
            }
        }
        assert_eq!(
            safe_fs::read(&temp.join("light/composition/sentinel.svg"), 1024).unwrap(),
            b"untouched light"
        );
        assert_eq!(live_store.live_hashes().unwrap(), before);
        assert_eq!(
            safe_fs::read(&target.skin.join("color/color_light.xml"), 65536).unwrap(),
            light_before
        );
        // Only the explicitly created temporary fixture is removed.
        std::fs::remove_dir_all(&temp).unwrap();
        println!("0.9.2.7: 31 sources; 9 in-memory palettes/opacity patches; 9 temp install/restore cycles; live files unchanged");
    }
    #[test]
    fn face_only_preserves_border_and_shadow_exactly() {
        let original = r##"<svg><g filter="url(#shadow)"><rect fill="white" fill-opacity="0.08"/></g><path d="test" fill="#151515"/><defs><filter id="shadow"/></defs></svg>"##;
        let out = background(original, "#151515", "#123456", 70).unwrap();
        assert_eq!(
            out,
            original.replace(
                r##"fill="#151515""##,
                r##"fill="#123456" fill-opacity="0.70""##
            )
        );
        assert!(background(&original.repeat(2), "#151515", "#123456", 100).is_err());
        assert!(background(
            &original.replace("#151515", "#161616"),
            "#151515",
            "#123456",
            100
        )
        .is_err());
        assert!(background(
            &original.replace(
                r##"fill="#151515""##,
                r##"fill="#151515" fill-opacity="0.9""##
            ),
            "#151515",
            "#123456",
            100
        )
        .is_err());
    }
    #[test]
    fn icon_preserves_hover_backing() {
        let original =
            r##"<svg><rect fill="white" fill-opacity="0.06"/><path fill="#5B8CFF"/></svg>"##;
        assert_eq!(
            icon(original, &theme::dark_colors()).unwrap(),
            original.replace("#5B8CFF", "#545454")
        );
        assert!(icon("<svg/>", &theme::dark_colors()).is_err());
    }
    #[test]
    fn managed_paths_never_touch_light_or_unrelated_windows() {
        let paths = paths();
        assert_eq!(paths.len(), 31);
        assert!(paths
            .iter()
            .all(|p| p == "window.xml" || p == "color/color_dark.xml" || p.starts_with("dark/")));
        assert!(!paths
            .iter()
            .any(|p| p.contains("menu") || p.contains("toast") || p.contains("white_bk")));
    }
}
