use anyhow::{bail, Context, Result};
use gpui_kit::base::dock::{DockAreaState, PanelInfo, PanelState};
use gpui_kit::Pixels;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const VERSION: u32 = 1;
pub const DEFAULTS: &[&str] = &["Research", "Code", "Focus"];

#[derive(Clone, Serialize, Deserialize)]
pub struct Layout {
    pub layout_version: u32,
    pub name: String,
    pub dock: DockAreaState,
}
impl Layout {
    pub fn new(name: &str, dock: DockAreaState) -> Self {
        Self {
            layout_version: VERSION,
            name: name.into(),
            dock,
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct Working {
    pub layout_version: u32,
    pub layout: Layout,
    pub saved: DockAreaState,
    pub active_module: String,
    #[serde(default)]
    pub arrangements: std::collections::BTreeMap<String, DockAreaState>,
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.trim() != name
        || name.is_empty()
        || name.len() > 80
        || name.starts_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        bail!("Use a name of 1–80 characters without slashes or leading dots.");
    }
    Ok(())
}
fn layout_path(name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    Ok(ados_core::paths::layouts_dir().join(format!("{name}.json")))
}
pub fn names() -> Vec<String> {
    let mut names: Vec<String> = DEFAULTS.iter().map(|s| s.to_string()).collect();
    if let Ok(entries) = fs::read_dir(ados_core::paths::layouts_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                if validate_name(name).is_ok() && !names.iter().any(|n| n == name) {
                    names.push(name.into());
                }
            }
        }
    }
    names
}
pub fn load(name: &str) -> Result<Layout> {
    let mut layout: Layout = serde_json::from_slice(&fs::read(layout_path(name)?)?)?;
    if layout.layout_version != VERSION {
        bail!("This layout has an unsupported version.");
    }
    if layout.name != name {
        bail!("The layout name does not match its file.");
    }
    drop_legacy_agent(&mut layout.dock);
    Ok(layout)
}
pub fn exists(name: &str) -> bool {
    layout_path(name).map(|p| p.exists()).unwrap_or(false)
}
pub fn save(layout: &Layout) -> Result<()> {
    atomic(&layout_path(&layout.name)?, layout)
}
pub fn working() -> Result<Option<Working>> {
    let path = ados_core::paths::state_file("shell");
    if !path.exists() {
        return Ok(None);
    }
    let mut state: Working = serde_json::from_slice(&fs::read(path)?)?;
    if state.layout_version != VERSION || state.layout.layout_version != VERSION {
        bail!("The workspace has an unsupported version.");
    }
    validate_name(&state.layout.name)?;
    drop_legacy_agent(&mut state.layout.dock);
    drop_legacy_agent(&mut state.saved);
    Ok(Some(state))
}
pub fn save_working(state: &Working) -> Result<()> {
    atomic(&ados_core::paths::state_file("shell"), state)
}
fn atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, bytes).with_context(|| format!("Cannot write {}", path.display()))?;
    fs::rename(&temp, path).with_context(|| format!("Cannot replace {}", path.display()))?;
    Ok(())
}

/// The assistant used to be a dock panel. Strip it before GPUI resolves panel names,
/// retaining split sizes and selected tabs for all surviving modules.
fn drop_legacy_agent(state: &mut DockAreaState) {
    fn prune(mut panel: PanelState) -> Option<PanelState> {
        if panel.panel_name == ados_core::ids::AGENT {
            return None;
        }
        if panel.children.is_empty() {
            return Some(panel);
        }
        let old_children = std::mem::take(&mut panel.children);
        let mut kept = Vec::new();
        for (ix, child) in old_children.into_iter().enumerate() {
            if let Some(child) = prune(child) {
                kept.push(ix);
                panel.children.push(child);
            }
        }
        if panel.children.is_empty() {
            return None;
        }
        match &mut panel.info {
            PanelInfo::Stack { sizes, .. } => {
                *sizes = kept
                    .iter()
                    .filter_map(|ix| sizes.get(*ix).copied())
                    .collect();
            }
            PanelInfo::Tabs { active_index } => {
                *active_index = kept.iter().position(|ix| ix == active_index).unwrap_or(0);
            }
            _ => {}
        }
        Some(panel)
    }
    state.center = prune(std::mem::take(&mut state.center)).unwrap_or_default();
    for dock in [
        &mut state.left_dock,
        &mut state.right_dock,
        &mut state.bottom_dock,
    ] {
        if let Some(old) = dock.take() {
            if let Some(panel) = prune(old.panel().clone()) {
                *dock = Some(gpui_kit::base::dock::DockState::new(
                    panel,
                    old.placement(),
                    old.size(),
                    old.open(),
                ));
            }
        }
    }
}

/// Fit absolute dock sizes to a viewport while preserving every split's ratios.
pub fn fit_to_viewport(state: &mut DockAreaState, width: Pixels, height: Pixels) {
    fn fit(panel: &mut PanelState, width: Pixels, height: Pixels) {
        let PanelInfo::Stack { sizes, axis } = &mut panel.info else {
            return;
        };
        if panel.children.is_empty() {
            return;
        }
        let available = if *axis == 0 { width } else { height };
        let total: f32 = sizes.iter().map(|size| f32::from(*size).max(1.)).sum();
        if sizes.len() != panel.children.len() || total <= 0. {
            *sizes = vec![available / panel.children.len() as f32; panel.children.len()];
        } else {
            for size in sizes.iter_mut() {
                *size = available * (f32::from(*size).max(1.) / total);
            }
        }
        for (child, size) in panel.children.iter_mut().zip(sizes) {
            fit(
                child,
                if *axis == 0 { *size } else { width },
                if *axis == 1 { *size } else { height },
            );
        }
    }
    fit(&mut state.center, width, height);
}

/// Module contents and tab focus are not a layout edit; split geometry and topology are.
pub fn same_arrangement(a: &DockAreaState, b: &DockAreaState) -> bool {
    fn same(a: &PanelState, b: &PanelState) -> bool {
        if a.panel_name != b.panel_name || a.children.len() != b.children.len() {
            return false;
        }
        match (&a.info, &b.info) {
            (PanelInfo::Stack { sizes: x, axis: ax }, PanelInfo::Stack { sizes: y, axis: ay }) => {
                if ax != ay
                    || x.len() != y.len()
                    || {
                        // Dock dumps measured pixels, which change on resize/zoom.
                        // Only a change in proportions is an arrangement edit.
                        let sx: f32 = x.iter().map(|v| f32::from(*v)).sum();
                        let sy: f32 = y.iter().map(|v| f32::from(*v)).sum();
                        if sx <= 0. || sy <= 0. {
                            x != y
                        } else {
                            x.iter().zip(y).any(|(a, b)|
                                (f32::from(*a) / sx - f32::from(*b) / sy).abs() > 0.005)
                        }
                    }
                {
                    return false;
                }
            }
            (PanelInfo::Tabs { .. }, PanelInfo::Tabs { .. })
            | (PanelInfo::Panel(_), PanelInfo::Panel(_)) => {}
            _ => return false,
        }
        a.children.iter().zip(&b.children).all(|(a, b)| same(a, b))
    }
    same(&a.center, &b.center)
        && a.left_dock == b.left_dock
        && a.right_dock == b.right_dock
        && a.bottom_dock == b.bottom_dock
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_agent_is_removed_with_empty_containers_and_tab_selection_remapped() {
        let agent = PanelState {
            panel_name: "agent".into(),
            ..Default::default()
        };
        let notes = PanelState {
            panel_name: "notes".into(),
            ..Default::default()
        };
        let tabs = PanelState {
            panel_name: "tabs".into(),
            children: vec![agent.clone(), notes.clone()],
            info: PanelInfo::tabs(1),
        };
        let empty = PanelState {
            panel_name: "tabs".into(),
            children: vec![agent],
            info: PanelInfo::tabs(0),
        };
        let mut state = DockAreaState {
            center: PanelState {
                panel_name: "stack".into(),
                children: vec![empty, tabs],
                info: PanelInfo::stack(vec![100.0.into(), 200.0.into()], gpui_kit::Axis::Horizontal),
            },
            ..Default::default()
        };
        drop_legacy_agent(&mut state);
        assert_eq!(state.center.children.len(), 1);
        assert_eq!(state.center.info, PanelInfo::stack(vec![200.0.into()], gpui_kit::Axis::Horizontal));
        let tabs = &state.center.children[0];
        assert_eq!(tabs.children, vec![notes]);
        assert_eq!(tabs.info, PanelInfo::tabs(0));
    }
    #[test]
    fn layout_names_cannot_escape_the_data_directory() {
        for name in [
            "",
            "../evil",
            "/tmp/x",
            "a/b",
            "a\\b",
            ".hidden",
            " padded",
            "bad\nname",
        ] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        for name in ["Research", "My writing space", "布局", "Code-2"] {
            assert!(validate_name(name).is_ok());
        }
    }
    fn split_fixture() -> DockAreaState {
        let mut state = DockAreaState::default();
        state.center.info = PanelInfo::Stack {
            sizes: vec![gpui_kit::px(300.), gpui_kit::px(100.)],
            axis: 0,
        };
        state.center.children = vec![
            PanelState {
                panel_name: "notes".into(),
                ..Default::default()
            },
            PanelState {
                panel_name: "browser".into(),
                ..Default::default()
            },
        ];
        state
    }
    #[test]
    fn resizing_a_restored_window_preserves_split_ratios() {
        let mut state = split_fixture();
        fit_to_viewport(&mut state, gpui_kit::px(800.), gpui_kit::px(600.));
        assert_eq!(
            state.center.info.sizes(),
            Some(&vec![gpui_kit::px(600.), gpui_kit::px(200.)])
        );
    }
    #[test]
    fn modified_means_arrangement_not_note_contents() {
        let saved = split_fixture();
        let mut state = saved.clone();
        state.center.children[0].info = PanelInfo::Panel(serde_json::json!({"text": "new draft"}));
        assert!(same_arrangement(&state, &saved));
        fit_to_viewport(&mut state, gpui_kit::px(800.), gpui_kit::px(600.));
        assert!(same_arrangement(&state, &saved));
        state.center.info = PanelInfo::Stack {
            sizes: vec![gpui_kit::px(250.), gpui_kit::px(150.)],
            axis: 0,
        };
        assert!(!same_arrangement(&state, &saved));
        state = saved.clone();
        state.center.children.swap(0, 1);
        assert!(!same_arrangement(&state, &saved));
    }
    #[test]
    fn old_working_state_gets_empty_arrangements() {
        let value = serde_json::json!({
            "layout_version": VERSION,
            "layout": Layout::new("Research", DockAreaState::default()),
            "saved": DockAreaState::default(), "active_module": "browser"
        });
        let restored: Working = serde_json::from_value(value).expect("legacy fixture");
        assert!(restored.arrangements.is_empty());
    }
    #[test]
    fn working_state_roundtrips_unsaved_arrangement() {
        let state = Working {
            layout_version: VERSION,
            layout: Layout::new("Research", DockAreaState::default()),
            saved: DockAreaState::default(),
            active_module: "browser".into(),
            arrangements: [("Code".into(), split_fixture())].into_iter().collect(),
        };
        let json = serde_json::to_vec(&state).expect("serialize fixture");
        let restored: Working = serde_json::from_slice(&json).expect("deserialize fixture");
        assert_eq!(restored.layout.name, "Research");
        assert_eq!(restored.layout.dock, state.layout.dock);
        assert_eq!(restored.saved, state.saved);
        assert_eq!(restored.active_module, "browser");
        assert_eq!(restored.arrangements["Code"], split_fixture());
    }
}
