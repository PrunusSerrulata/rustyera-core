use std::cmp::Ordering;

use era_runtime_protocol::{
    PresentationOperation, ResourceReplay, ResourceReplayDelta, ResourceReplayListEdit,
};

pub(super) fn resource_update(
    before: &ResourceReplay,
    after: &ResourceReplay,
) -> Option<PresentationOperation> {
    if before == after {
        return None;
    }
    let full = || PresentationOperation::SetResources {
        resources: after.clone(),
    };
    let Some(sprite_edits) = list_edits(&before.sprites, &after.sprites, |a, b| {
        a.name.cmp(&b.name).then(a.revision.cmp(&b.revision))
    }) else {
        return Some(full());
    };
    let Some(canvas_edits) = list_edits(&before.canvases, &after.canvases, |a, b| {
        a.canvas_id
            .cmp(&b.canvas_id)
            .then(a.revision.cmp(&b.revision))
    }) else {
        return Some(full());
    };
    let changed = edit_weight(&sprite_edits).saturating_add(edit_weight(&canvas_edits));
    // Cold or near-total replacement has no useful shared baseline. Avoid building JSON merely
    // to choose an encoding; the resource-entry count is a conservative cheap decision metric.
    if changed >= after.sprites.len().saturating_add(after.canvases.len()) {
        return Some(full());
    }
    Some(PresentationOperation::ApplyResourceDelta {
        delta: ResourceReplayDelta {
            sprite_edits,
            canvas_edits,
            animation_timer_ms: after.animation_timer_ms,
        },
    })
}

fn edit_weight<T>(edits: &[ResourceReplayListEdit<T>]) -> usize {
    edits.iter().fold(0_usize, |sum, edit| {
        sum.saturating_add((edit.delete_count as usize).max(edit.insert.len()))
    })
}

/// Linear merge of canonical lists. Unsorted legacy/custom baselines use a full replacement
/// instead, preserving their exact order rather than inventing a new canonical state.
fn list_edits<T: Clone + Eq>(
    before: &[T],
    after: &[T],
    compare: impl Fn(&T, &T) -> Ordering,
) -> Option<Vec<ResourceReplayListEdit<T>>> {
    u32::try_from(before.len()).ok()?;
    u32::try_from(after.len()).ok()?;
    if !before
        .windows(2)
        .all(|pair| compare(&pair[0], &pair[1]).is_lt())
        || !after
            .windows(2)
            .all(|pair| compare(&pair[0], &pair[1]).is_lt())
    {
        return None;
    }
    let mut edits = Vec::new();
    let (mut left, mut right) = (0, 0);
    while left < before.len() && right < after.len() {
        match compare(&before[left], &after[right]) {
            Ordering::Equal => {
                if before[left] != after[right] {
                    push_edit(&mut edits, left, 1, Some(after[right].clone()));
                }
                left += 1;
                right += 1;
            }
            Ordering::Less => {
                push_edit(&mut edits, left, 1, None);
                left += 1;
            }
            Ordering::Greater => {
                push_edit(&mut edits, left, 0, Some(after[right].clone()));
                right += 1;
            }
        }
    }
    if left < before.len() {
        push_edit(&mut edits, left, before.len() - left, None);
        left = before.len();
    }
    for value in &after[right..] {
        push_edit(&mut edits, left, 0, Some(value.clone()));
    }
    Some(edits)
}

fn push_edit<T>(
    edits: &mut Vec<ResourceReplayListEdit<T>>,
    start: usize,
    delete_count: usize,
    inserted: Option<T>,
) {
    if let Some(last) = edits.last_mut()
        && last.start as usize + last.delete_count as usize == start
    {
        last.delete_count += u32::try_from(delete_count).expect("list length was checked");
        last.insert.extend(inserted);
        return;
    }
    edits.push(ResourceReplayListEdit {
        start: u32::try_from(start).expect("list length was checked"),
        delete_count: u32::try_from(delete_count).expect("list length was checked"),
        insert: inserted.into_iter().collect(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use era_runtime_protocol::SpriteReplay;

    fn sprite(index: usize) -> SpriteReplay {
        SpriteReplay {
            name: format!("SPRITE{index:06}"),
            size: [1, 1],
            position: [0, 0],
            frames: Vec::new(),
            canvas_id: None,
            canvas_rectangle: None,
            revision: 1,
            canvas_revision: None,
        }
    }

    fn replay(mask: usize) -> ResourceReplay {
        ResourceReplay {
            sprites: (0..4)
                .filter(|index| mask & (1 << index) != 0)
                .map(sprite)
                .collect(),
            ..ResourceReplay::default()
        }
    }

    fn reconstructed(before: &ResourceReplay, after: &ResourceReplay) -> ResourceReplay {
        match resource_update(before, after) {
            None => before.clone(),
            Some(PresentationOperation::SetResources { resources }) => resources,
            Some(PresentationOperation::ApplyResourceDelta { delta }) => {
                before.apply_delta(&delta).unwrap()
            }
            _ => panic!("unexpected resource operation"),
        }
    }

    #[test]
    fn resource_list_edits_preserve_every_small_insert_delete_and_replace_combination() {
        for left in 0..16 {
            for right in 0..16 {
                let before = replay(left);
                let mut after = replay(right);
                assert_eq!(reconstructed(&before, &after), after);
                for entry in &mut after.sprites {
                    entry.position = [5, 7];
                }
                // Payload changes must not be missed even if a producer kept the same revision.
                assert_eq!(reconstructed(&before, &after), after);
            }
        }
    }

    #[test]
    fn large_unchanged_resource_catalog_only_sends_small_edits() {
        let before = ResourceReplay {
            sprites: (0..28_000).map(sprite).collect(),
            ..ResourceReplay::default()
        };
        let mut after = before.clone();
        after.sprites[14_000].revision = 2;
        after.sprites.push(sprite(28_000));
        let Some(PresentationOperation::ApplyResourceDelta { delta }) =
            resource_update(&before, &after)
        else {
            panic!("small changes must use a delta");
        };
        assert_eq!(
            delta
                .sprite_edits
                .iter()
                .map(|edit| edit.insert.len())
                .sum::<usize>(),
            2
        );
        assert!(serde_json::to_vec(&delta).unwrap().len() < 2048);
        assert_eq!(before.apply_delta(&delta).unwrap(), after);
    }

    #[test]
    fn resources_keep_historical_revisions_timer_and_unsorted_fallback() {
        let mut before = replay(15);
        let mut historical = before.sprites[1].clone();
        historical.revision = 2;
        before.sprites.insert(2, historical);
        let mut after = before.clone();
        after.sprites.remove(1);
        after.animation_timer_ms = -1;
        assert_eq!(reconstructed(&before, &after), after);
        let mut timer_only = before.clone();
        timer_only.animation_timer_ms = 50;
        assert_eq!(reconstructed(&before, &timer_only), timer_only);
        after.sprites.reverse();
        assert!(matches!(
            resource_update(&before, &after),
            Some(PresentationOperation::SetResources { .. })
        ));
        assert_eq!(reconstructed(&before, &after), after);
        assert!(resource_update(&before, &before).is_none());
    }
}
