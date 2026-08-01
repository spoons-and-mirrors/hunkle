use super::*;
use tempfile::TempDir;

fn paths(temp: &TempDir) -> ThemePaths {
    ThemePaths::new(
        temp.path().join("config/opencode"),
        temp.path().join("state/opencode/kv.json"),
        temp.path().join("work/repository"),
    )
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("test path has a parent")).unwrap();
    fs::write(path, contents).unwrap();
}

fn custom_theme(primary: &str) -> String {
    format!(
        r##"{{
            "defs": {{
                "base": "#010203",
                "panel": {{ "dark": "#040506", "light": "#a0a1a2" }},
                "shared": "base"
            }},
            "theme": {{
                "primary": {primary},
                "secondary": "#101112",
                "accent": "#131415",
                "error": "#161718",
                "warning": "#191a1b",
                "success": "#1c1d1e",
                "info": "#1f2021",
                "text": "shared",
                "textMuted": 42,
                "background": "transparent",
                "backgroundPanel": "panel",
                "backgroundElement": "#252627",
                "border": "#28292a",
                "borderActive": "#2b2c2d",
                "borderSubtle": "#2e2f30",
                "diffHunkHeader": "#313233",
                "diffAddedBg": "#343536",
                "diffRemovedBg": "#373839"
            }}
        }}"##
    )
}

#[test]
fn unavailable_selection_uses_exact_default() {
    let temp = TempDir::new().unwrap();
    let loaded = ThemeLoader::new(paths(&temp)).load();

    assert_eq!(loaded.name, DEFAULT_THEME_NAME);
    assert_eq!(loaded.source, ThemeSource::Fallback);
    assert_eq!(loaded.palette, default_palette());
    assert_eq!(loaded.palette.accent, Color::Rgb(0x8a, 0xad, 0xf4));
    assert_eq!(loaded.palette.canvas, Color::Rgb(0x24, 0x27, 0x3a));
    assert_eq!(loaded.palette.add_bg, Color::Rgb(0x29, 0x34, 0x2b));
}

#[test]
fn kv_selects_registered_builtin() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    write(&paths.state_file, r#"{"theme":"catppuccin-macchiato"}"#);

    let loaded = ThemeLoader::new(paths).load();

    assert_eq!(loaded.source, ThemeSource::BuiltIn);
    assert_eq!(loaded.palette, default_palette());
}

#[test]
fn kv_theme_mode_lock_selects_light_appearance() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    write(
        &paths.state_file,
        r#"{"theme":"catppuccin","theme_mode_lock":"light"}"#,
    );

    assert_eq!(selected_appearance(&paths), Appearance::Light);
}

#[test]
fn every_official_builtin_resolves_with_distinct_representatives() {
    const NAMES: &[&str] = &[
        "aura",
        "ayu",
        "carbonfox",
        "catppuccin-frappe",
        "catppuccin-macchiato",
        "catppuccin",
        "cobalt2",
        "cursor",
        "dracula",
        "everforest",
        "flexoki",
        "github",
        "gruvbox",
        "kanagawa",
        "lucent-orng",
        "material",
        "matrix",
        "mercury",
        "monokai",
        "nightowl",
        "nord",
        "one-dark",
        "opencode",
        "orng",
        "osaka-jade",
        "palenight",
        "rosepine",
        "solarized",
        "synthwave84",
        "tokyonight",
        "vercel",
        "vesper",
        "zenburn",
    ];
    assert_eq!(theme_builtins::names().collect::<Vec<_>>(), NAMES);

    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    let mut representatives = Vec::new();
    for name in NAMES {
        write(&paths.state_file, &format!(r#"{{"theme":"{name}"}}"#));
        let dark = ThemeLoader::new(paths.clone()).load();
        let light = ThemeLoader::new(paths.clone())
            .appearance(Appearance::Light)
            .load();

        assert_eq!(dark.name, *name);
        assert_eq!(dark.source, ThemeSource::BuiltIn);
        assert_eq!(light.source, ThemeSource::BuiltIn);
        if matches!(*name, "catppuccin-macchiato" | "gruvbox" | "tokyonight") {
            representatives.push(dark.palette.accent);
        }
    }

    assert_eq!(representatives.len(), 3);
    assert_ne!(representatives[0], representatives[1]);
    assert_ne!(representatives[0], representatives[2]);
    assert_ne!(representatives[1], representatives[2]);
}

#[test]
fn jsonc_config_overrides_kv_and_preserves_comment_like_strings() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    write(&paths.state_file, r#"{"theme":"catppuccin-macchiato"}"#);
    write(
        &paths.config_dir.join("tui.jsonc"),
        r#"{
            "$schema": "https://opencode.ai/tui.json", // comment
            "theme": "custom", /* another comment */
        }"#,
    );
    let theme_path = paths.config_dir.join("themes/custom.json");
    write(&theme_path, &custom_theme(r##""#abcdef""##));

    let loaded = ThemeLoader::new(paths).load();

    assert_eq!(loaded.source, ThemeSource::User(theme_path));
    assert_eq!(loaded.palette.accent, Color::Rgb(0xab, 0xcd, 0xef));
}

#[test]
fn nearest_project_theme_resolves_refs_variants_ansi_and_transparency() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    write(&paths.state_file, r#"{"theme":"custom"}"#);
    let user_theme = paths.config_dir.join("themes/custom.json");
    write(&user_theme, &custom_theme(r##""#ffffff""##));
    let ancestor_theme = temp.path().join("work/.opencode/themes/custom.json");
    write(&ancestor_theme, &custom_theme(r##""#999999""##));
    let project_theme = paths.cwd.join(".opencode/themes/custom.json");
    write(&project_theme, &custom_theme(r##""#abcdef""##));

    let dark = ThemeLoader::new(paths.clone()).load();
    let light = ThemeLoader::new(paths).appearance(Appearance::Light).load();

    assert_eq!(dark.source, ThemeSource::Project(project_theme.clone()));
    assert_eq!(dark.palette.canvas, Color::Reset);
    assert_eq!(dark.palette.ink, Color::Rgb(1, 2, 3));
    assert_eq!(dark.palette.muted, Color::Indexed(42));
    assert_eq!(dark.palette.panel, Color::Rgb(4, 5, 6));
    assert_eq!(dark.palette.accent, Color::Rgb(0xab, 0xcd, 0xef));
    assert_eq!(light.source, ThemeSource::Project(project_theme));
    assert_eq!(light.palette.panel, Color::Rgb(0xa0, 0xa1, 0xa2));
}

#[test]
fn malformed_config_or_custom_theme_uses_exact_default() {
    let temp = TempDir::new().unwrap();
    let mut paths = paths(&temp);
    write(&paths.state_file, r#"{"theme":"catppuccin-macchiato"}"#);
    write(&paths.config_dir.join("tui.jsonc"), "{/* unclosed");
    assert_eq!(ThemeLoader::new(paths.clone()).load(), fallback_theme());

    fs::remove_file(paths.config_dir.join("tui.jsonc")).unwrap();
    write(&paths.state_file, r#"{"theme":"broken"}"#);
    paths.cwd = temp.path().join("work");
    write(
        &paths.cwd.join(".opencode/themes/broken.json"),
        &custom_theme(r#""missing-reference""#),
    );
    assert_eq!(ThemeLoader::new(paths).load(), fallback_theme());
}

#[test]
fn cyclic_references_use_exact_default() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    write(&paths.state_file, r#"{"theme":"cyclic"}"#);
    write(
        &paths.config_dir.join("themes/cyclic.json"),
        &custom_theme(r#""primary""#),
    );

    assert_eq!(ThemeLoader::new(paths).load(), fallback_theme());
}

#[test]
fn same_named_defs_and_upstream_hex_forms_resolve() {
    let document = serde_json::json!({
        "defs": { "primary": "#abc" },
        "theme": { "primary": "primary" }
    });
    let resolver = Resolver {
        defs: document["defs"].as_object().unwrap(),
        theme: document["theme"].as_object().unwrap(),
        appearance: Appearance::Dark,
    };

    assert_eq!(resolver.key("primary"), Ok(Color::Rgb(0xaa, 0xbb, 0xcc)));
    assert_eq!(parse_hex("#1230"), Some(Color::Reset));
    assert_eq!(parse_hex("#12345680"), Some(Color::Rgb(0x12, 0x34, 0x56)));
}
