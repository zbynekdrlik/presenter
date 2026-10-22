//! Plate ORDER logic (#871), pure and host-tested: the ↑ / ↓ step, the drag
//! drop slot and its marker, and applying a new order to the local list.
//!
//! The server's reorder (`PUT …/nameplates/order`) wants the FULL id set, so
//! every new order here holds every id exactly once. A plate's position is its
//! Stream Deck button (#870), so the order is what the operator rearranges.

use presenter_core::Nameplate;

/// The id list after moving `id` one step up or down, or `None` when it cannot
/// move (already first / last, or not in the list).
pub fn reorder_one_step(ids: &[i64], id: i64, up: bool) -> Option<Vec<i64>> {
    let pos = ids.iter().position(|x| *x == id)?;
    let target = if up {
        pos.checked_sub(1)?
    } else if pos + 1 < ids.len() {
        pos + 1
    } else {
        return None;
    };
    let mut moved = ids.to_vec();
    moved.swap(pos, target);
    Some(moved)
}

/// The drop slot under the pointer: how many rows have their vertical middle
/// above `y` (0 = before the first row, `len` = after the last). `middles` are
/// the rows' middles in list order, the dragged row included.
pub fn drop_slot(middles: &[f64], y: f64) -> usize {
    middles.iter().filter(|middle| **middle < y).count()
}

/// The id list after dropping `id` into `slot` (a [`drop_slot`] value), or
/// `None` when that leaves the order unchanged (a drop on the row's own place)
/// or `id` is not in the list.
pub fn reorder_to_slot(ids: &[i64], id: i64, slot: usize) -> Option<Vec<i64>> {
    let from = ids.iter().position(|x| *x == id)?;
    let mut moved: Vec<i64> = ids.iter().copied().filter(|x| *x != id).collect();
    // Removing the row first shifts every slot below it up by one.
    let insert = if slot > from { slot - 1 } else { slot }.min(moved.len());
    moved.insert(insert, id);
    (moved != ids).then_some(moved)
}

/// Where the drop line shows on the row at `pos` while the row at `from` is
/// dragged to `slot` in a list of `len`: `"before"` the row, `"after"` it (the
/// last row only), or nowhere. A slot next to the dragged row's own place
/// would not move it, so it shows no line at all.
pub fn drop_marker(from: usize, slot: usize, pos: usize, len: usize) -> Option<&'static str> {
    if slot == from || slot == from + 1 {
        None
    } else if slot == pos {
        Some("before")
    } else if slot == len && pos + 1 == len {
        Some("after")
    } else {
        None
    }
}

/// Put the local list into the order of `ids` (an order the page just chose,
/// shown at once while the PUT is on its way). Plates missing from `ids` keep
/// their relative order at the end.
pub fn apply_order(list: &mut [Nameplate], ids: &[i64]) {
    list.sort_by_key(|plate| {
        ids.iter()
            .position(|id| *id == plate.id)
            .unwrap_or(usize::MAX)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::NameplateKind;

    #[test]
    fn a_plate_moves_one_step_and_keeps_the_full_id_set() {
        assert_eq!(reorder_one_step(&[1, 2, 3], 2, true), Some(vec![2, 1, 3]));
        assert_eq!(reorder_one_step(&[1, 2, 3], 2, false), Some(vec![1, 3, 2]));
        assert_eq!(reorder_one_step(&[1, 2, 3], 3, true), Some(vec![1, 3, 2]));
    }

    #[test]
    fn the_first_cannot_go_up_the_last_cannot_go_down_an_unknown_id_not_at_all() {
        assert_eq!(reorder_one_step(&[1, 2, 3], 1, true), None);
        assert_eq!(reorder_one_step(&[1, 2, 3], 3, false), None);
        assert_eq!(reorder_one_step(&[1, 2, 3], 9, true), None);
        assert_eq!(reorder_one_step(&[], 1, false), None);
    }

    #[test]
    fn the_drop_slot_counts_the_rows_above_the_pointer() {
        let middles = [10.0, 30.0, 50.0];
        assert_eq!(drop_slot(&middles, -100.0), 0);
        assert_eq!(drop_slot(&middles, 20.0), 1);
        assert_eq!(drop_slot(&middles, 45.0), 2);
        assert_eq!(drop_slot(&middles, 500.0), 3);
        assert_eq!(drop_slot(&[], 20.0), 0);
    }

    #[test]
    fn a_drop_above_the_first_below_the_last_or_in_the_middle_moves_the_plate() {
        let ids = [1, 2, 3, 4];
        assert_eq!(reorder_to_slot(&ids, 3, 0), Some(vec![3, 1, 2, 4]));
        assert_eq!(reorder_to_slot(&ids, 1, 4), Some(vec![2, 3, 4, 1]));
        assert_eq!(reorder_to_slot(&ids, 4, 2), Some(vec![1, 2, 4, 3]));
        assert_eq!(reorder_to_slot(&ids, 1, 3), Some(vec![2, 3, 1, 4]));
    }

    #[test]
    fn a_drop_on_the_rows_own_place_or_of_an_unknown_id_changes_nothing() {
        let ids = [1, 2, 3];
        assert_eq!(reorder_to_slot(&ids, 2, 1), None);
        assert_eq!(reorder_to_slot(&ids, 2, 2), None);
        assert_eq!(reorder_to_slot(&ids, 9, 0), None);
        assert_eq!(reorder_to_slot(&[1], 1, 0), None);
        assert_eq!(reorder_to_slot(&[1], 1, 1), None);
        assert_eq!(reorder_to_slot(&[], 1, 0), None);
    }

    #[test]
    fn the_drop_line_shows_only_where_the_plate_would_land() {
        // Row 1 of 4 dragged.
        assert_eq!(drop_marker(1, 1, 1, 4), None);
        assert_eq!(drop_marker(1, 2, 2, 4), None);
        assert_eq!(drop_marker(1, 0, 0, 4), Some("before"));
        assert_eq!(drop_marker(1, 3, 3, 4), Some("before"));
        assert_eq!(drop_marker(1, 4, 3, 4), Some("after"));
        assert_eq!(drop_marker(1, 4, 2, 4), None);
        assert_eq!(drop_marker(1, 0, 2, 4), None);
    }

    fn plate(id: i64) -> Nameplate {
        Nameplate {
            id,
            kind: NameplateKind::Person,
            primary_text: format!("p{id}"),
            secondary_text: String::new(),
            sort_order: 0,
        }
    }

    #[test]
    fn the_local_list_takes_the_new_order() {
        let mut list = vec![plate(1), plate(2), plate(3)];
        apply_order(&mut list, &[3, 1, 2]);
        assert_eq!(list.iter().map(|p| p.id).collect::<Vec<_>>(), vec![3, 1, 2]);
        apply_order(&mut list, &[2]);
        assert_eq!(list.iter().map(|p| p.id).collect::<Vec<_>>(), vec![2, 3, 1]);
    }
}
