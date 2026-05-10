use crate::config::schema::{
    AppConfig, OverlayItem, CURRENT_SCHEMA_VERSION,
};

/// Clamp out-of-range fields and fix obvious mistakes in user-edited configs.
/// Returns true if any value was changed and should be persisted back.
pub fn normalize(cfg: &mut AppConfig) -> bool {
    let original = cfg.clone();

    if cfg.schema_version < CURRENT_SCHEMA_VERSION {
        cfg.schema_version = CURRENT_SCHEMA_VERSION;
    }

    cfg.general.sample_interval_ms = cfg.general.sample_interval_ms.clamp(500, 5000);
    cfg.history.retain_days = cfg.history.retain_days.clamp(31, 3650);

    if cfg.overlay.items.is_empty() {
        cfg.overlay.items = AppConfig::default().overlay.items;
    }
    normalize_overlay_item_groups(&mut cfg.overlay.items);

    *cfg != original
}

fn normalize_overlay_item_groups(items: &mut Vec<OverlayItem>) {
    replace_item(items, OverlayItem::Gpu, OverlayItem::GpuUsage);
    ensure_pair(items, OverlayItem::NetDown, OverlayItem::NetUp);
    ensure_pair(items, OverlayItem::DiskRead, OverlayItem::DiskWrite);
}

fn replace_item(items: &mut Vec<OverlayItem>, old: OverlayItem, new: OverlayItem) {
    let had_old = items.iter().any(|item| *item == old);
    items.retain(|item| *item != old);
    if had_old && !items.contains(&new) {
        items.push(new);
    }
}

fn ensure_pair(items: &mut Vec<OverlayItem>, first: OverlayItem, second: OverlayItem) {
    let first_pos = items.iter().position(|item| *item == first);
    let second_pos = items.iter().position(|item| *item == second);
    match (first_pos, second_pos) {
        (Some(i), None) => items.insert(i + 1, second),
        (None, Some(i)) => items.insert(i, first),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_upgrades_schema_and_keeps_overlay_items_only() {
        let mut cfg = AppConfig::default();
        cfg.schema_version = 3;
        cfg.overlay.items = vec![OverlayItem::Gpu, OverlayItem::DiskWrite];

        assert!(normalize(&mut cfg));

        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            cfg.overlay.items,
            vec![
                OverlayItem::DiskRead,
                OverlayItem::DiskWrite,
                OverlayItem::GpuUsage,
            ]
        );
    }
}
