//! Comprehensive integration and verification test suite for `DocCommand::SliceStroke`
//! and `PageSpatialIndex::remove_item` (UniNote Phase 4B Part 2 - Milestone M2).
//!
//! Covers:
//! 1. Complete deletion (`new_strokes: vec![]`)
//! 2. Single trimmed stroke (`new_strokes: vec![trimmed]`)
//! 3. Bisection into 2 fragments (`new_strokes: vec![s1, s2]`)
//! 4. Multi-split into 3+ fragments (`new_strokes: vec![s1, s2, s3, s4]`)
//! 5. 1,000 randomized Undo/Redo cycles checking spatial index invariants
//! 6. Nested hierarchical composition (slice S -> S1, S2, then S1 -> S1a, S1b, undo twice, redo twice)
//! 7. Culling query fidelity (`locate_in_envelope` over fragment bounds vs original stroke)
//! 8. Serde roundtrip of `DocCommand::SliceStroke`
//! 9. Direct verification of `PageSpatialIndex::remove_item` including fallback behavior

use std::collections::HashSet;
use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::model::{Document, Item, ItemKind, Page};
use uninote_core::spatial::PageSpatialIndex;
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

/// Helper to generate a stroke with points and recomputed cache.
fn make_test_stroke(id: u64, pts: &[(f64, f64)]) -> Stroke {
    let mut stroke = Stroke::new(id, StrokeStyle::default(), PipelineMode::CatmullRomSpline);
    for (i, &(x, y)) in pts.iter().enumerate() {
        stroke.points.push(StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 3.0,
            timestamp_ns: (i as u64 + 1) * 1000,
        });
    }
    stroke.recompute_cache();
    stroke
}

/// Helper to wrap a Stroke into an Item.
fn make_item(id: u64, z_index: u64, stroke: Stroke) -> Item {
    Item {
        id,
        z_index,
        kind: ItemKind::Stroke(stroke),
    }
}

/// Helper to assert spatial index invariant:
/// Spatial index count matches items with non-empty bounding box,
/// and all items in `page.items` are uniquely queryable in `page.spatial_index`.
fn assert_spatial_index_invariant(page: &Page) {
    let items_with_bbox_count = page
        .items
        .iter()
        .filter(|it| it.bounding_box().is_some())
        .count();

    assert_eq!(
        page.spatial_index.len(),
        items_with_bbox_count,
        "Spatial index len ({}) must match items with bbox ({})",
        page.spatial_index.len(),
        items_with_bbox_count
    );

    let mut id_set = HashSet::new();
    for it in &page.items {
        assert!(
            id_set.insert(it.id),
            "Duplicate item id {} found in page.items",
            it.id
        );

        if let Some(bbox) = it.bounding_box() {
            let found_in_tree = page
                .spatial_index
                .tree()
                .iter()
                .any(|spatial_item| spatial_item.item_id == it.id);
            assert!(
                found_in_tree,
                "Item {} with bbox {:?} missing from spatial index tree",
                it.id, bbox
            );
        }
    }

    for spatial_item in page.spatial_index.tree().iter() {
        assert!(
            id_set.contains(&spatial_item.item_id),
            "Spatial item id {} in tree not present in page.items",
            spatial_item.item_id
        );
    }
}

#[test]
fn test_slice_stroke_complete_deletion_undo_redo() {
    let mut doc = Document::new_notebook("Test Complete Deletion");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    // Initial strokes: A (z=1), B (z=2), C (z=3)
    let s_a = make_test_stroke(0, &[(10.0, 10.0), (30.0, 30.0)]);
    let s_b = make_test_stroke(0, &[(50.0, 50.0), (70.0, 70.0)]);
    let s_c = make_test_stroke(0, &[(90.0, 90.0), (110.0, 110.0)]);

    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_a,
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_b,
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_c,
        },
        &mut doc,
    );

    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 3);
    assert_spatial_index_invariant(page);

    let id_a = page.items[0].id;
    let orig_b = page.items[1].clone();
    let id_b = orig_b.id;
    let id_c = page.items[2].id;

    // Slice B completely: new_strokes is empty
    let slice_cmd = DocCommand::SliceStroke {
        page_id,
        original_stroke: orig_b.clone(),
        new_strokes: Vec::new(),
    };

    history.execute(slice_cmd, &mut doc);

    // Post-execution assertions
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].id, id_a);
    assert_eq!(page.items[1].id, id_c);
    assert_spatial_index_invariant(page);

    let query_b = page
        .spatial_index
        .locate_in_envelope(40.0, 40.0, 80.0, 80.0);
    assert!(
        query_b.is_empty(),
        "Erased stroke B must not be in spatial index"
    );

    // UNDO: B must be restored in correct position and z_index
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.items[0].id, id_a);
    assert_eq!(page.items[1].id, id_b);
    assert_eq!(page.items[2].id, id_c);
    assert_eq!(page.items[1].z_index, orig_b.z_index);
    assert_spatial_index_invariant(page);

    let query_b_restored = page
        .spatial_index
        .locate_in_envelope(40.0, 40.0, 80.0, 80.0);
    assert_eq!(query_b_restored.len(), 1);
    assert_eq!(query_b_restored[0].item_id, id_b);

    // REDO: B must be removed again
    assert!(history.redo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 2);
    assert_spatial_index_invariant(page);

    // UNDO again
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 3);
    assert_spatial_index_invariant(page);
}

#[test]
fn test_slice_stroke_trimmed_endpoint_undo_redo() {
    let mut doc = Document::new_notebook("Test Trimmed Stroke");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    let s_orig = make_test_stroke(0, &[(100.0, 100.0), (150.0, 150.0), (200.0, 200.0)]);
    let s_other = make_test_stroke(0, &[(300.0, 300.0), (350.0, 350.0)]);

    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_orig,
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_other,
        },
        &mut doc,
    );

    let orig_item = doc.pages[0].items[0].clone();
    let other_item = doc.pages[0].items[1].clone();
    let orig_id = orig_item.id;
    let other_id = other_item.id;

    // Trimmed stroke has end chopped off: (100.0, 100.0) to (140.0, 140.0)
    let trimmed_id = history.alloc_item_id();
    let s_trimmed = make_test_stroke(trimmed_id, &[(100.0, 100.0), (140.0, 140.0)]);
    let trimmed_item = make_item(trimmed_id, orig_item.z_index, s_trimmed);

    let slice_cmd = DocCommand::SliceStroke {
        page_id,
        original_stroke: orig_item.clone(),
        new_strokes: vec![trimmed_item],
    };

    history.execute(slice_cmd, &mut doc);

    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 2);
    assert_eq!(
        page.items[0].id, trimmed_id,
        "Trimmed stroke replaced original in-place"
    );
    assert_eq!(page.items[1].id, other_id);
    assert_spatial_index_invariant(page);

    // Region (160..210) should no longer contain any stroke
    let query_trimmed_tail = page
        .spatial_index
        .locate_in_envelope(160.0, 160.0, 210.0, 210.0);
    assert!(query_trimmed_tail.is_empty());

    // UNDO: original restored
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].id, orig_id);
    assert_eq!(page.items[1].id, other_id);
    assert_spatial_index_invariant(page);

    let query_tail_restored = page
        .spatial_index
        .locate_in_envelope(160.0, 160.0, 210.0, 210.0);
    assert_eq!(query_tail_restored.len(), 1);
    assert_eq!(query_tail_restored[0].item_id, orig_id);

    // REDO: trimmed restored
    assert!(history.redo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items[0].id, trimmed_id);
    assert_spatial_index_invariant(page);
}

#[test]
fn test_slice_stroke_split_two_fragments_undo_redo() {
    let mut doc = Document::new_notebook("Test Bisection");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    let s_before = make_test_stroke(0, &[(10.0, 10.0), (20.0, 20.0)]);
    let s_target = make_test_stroke(0, &[(100.0, 100.0), (200.0, 100.0)]);
    let s_after = make_test_stroke(0, &[(500.0, 500.0), (600.0, 600.0)]);

    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_before,
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_target,
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_after,
        },
        &mut doc,
    );

    let item_before = doc.pages[0].items[0].clone();
    let orig_item = doc.pages[0].items[1].clone();
    let item_after = doc.pages[0].items[2].clone();

    // Bisect target at x=150: frag1 (100..140), frag2 (160..200)
    let id_frag1 = history.alloc_item_id();
    let id_frag2 = history.alloc_item_id();
    let s_frag1 = make_test_stroke(id_frag1, &[(100.0, 100.0), (140.0, 100.0)]);
    let s_frag2 = make_test_stroke(id_frag2, &[(160.0, 100.0), (200.0, 100.0)]);

    let frag1_item = make_item(id_frag1, orig_item.z_index, s_frag1);
    let frag2_item = make_item(id_frag2, orig_item.z_index, s_frag2);

    let slice_cmd = DocCommand::SliceStroke {
        page_id,
        original_stroke: orig_item.clone(),
        new_strokes: vec![frag1_item, frag2_item],
    };

    history.execute(slice_cmd, &mut doc);

    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 4);
    assert_eq!(page.items[0].id, item_before.id);
    assert_eq!(page.items[1].id, id_frag1);
    assert_eq!(page.items[2].id, id_frag2);
    assert_eq!(page.items[3].id, item_after.id);
    assert_spatial_index_invariant(page);

    // Query gap x in [145, 155]: nothing hit
    let gap_query = page
        .spatial_index
        .locate_in_envelope(145.0, 95.0, 155.0, 105.0);
    assert!(gap_query.is_empty(), "Eraser gap must have no hits");

    // Query frag1
    let q1 = page
        .spatial_index
        .locate_in_envelope(95.0, 95.0, 120.0, 105.0);
    assert_eq!(q1.len(), 1);
    assert_eq!(q1[0].item_id, id_frag1);

    // Query frag2
    let q2 = page
        .spatial_index
        .locate_in_envelope(170.0, 95.0, 190.0, 105.0);
    assert_eq!(q2.len(), 1);
    assert_eq!(q2[0].item_id, id_frag2);

    // UNDO: original restored in exact slot
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.items[0].id, item_before.id);
    assert_eq!(page.items[1].id, orig_item.id);
    assert_eq!(page.items[2].id, item_after.id);
    assert_spatial_index_invariant(page);

    // Gap query now hits original stroke
    let gap_query_undo = page
        .spatial_index
        .locate_in_envelope(145.0, 95.0, 155.0, 105.0);
    assert_eq!(gap_query_undo.len(), 1);
    assert_eq!(gap_query_undo[0].item_id, orig_item.id);

    // REDO: bisection restored
    assert!(history.redo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 4);
    assert_eq!(page.items[1].id, id_frag1);
    assert_eq!(page.items[2].id, id_frag2);
    assert_spatial_index_invariant(page);
}

#[test]
fn test_slice_stroke_multi_split_three_or_more() {
    let mut doc = Document::new_notebook("Test Multi-Split");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    let s_orig = make_test_stroke(0, &[(0.0, 0.0), (1000.0, 0.0)]);
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_orig,
        },
        &mut doc,
    );

    let orig_item = doc.pages[0].items[0].clone();
    let frag_ids: Vec<u64> = (0..4).map(|_| history.alloc_item_id()).collect();

    // Multi-cut into 4 pieces
    let frags: Vec<Item> = (0..4)
        .map(|idx| {
            let start_x = idx as f64 * 250.0;
            let end_x = start_x + 200.0;
            let s = make_test_stroke(frag_ids[idx], &[(start_x, 0.0), (end_x, 0.0)]);
            make_item(frag_ids[idx], orig_item.z_index, s)
        })
        .collect();

    let slice_cmd = DocCommand::SliceStroke {
        page_id,
        original_stroke: orig_item.clone(),
        new_strokes: frags.clone(),
    };

    history.execute(slice_cmd, &mut doc);

    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 4);
    for (idx, &expected_id) in frag_ids.iter().enumerate() {
        assert_eq!(page.items[idx].id, expected_id);
    }
    assert_spatial_index_invariant(page);

    // UNDO: single original stroke restored
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, orig_item.id);
    assert_spatial_index_invariant(page);

    // REDO: all 4 fragments restored
    assert!(history.redo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 4);
    assert_spatial_index_invariant(page);
}

#[test]
fn test_slice_stroke_nested_hierarchical_composition() {
    let mut doc = Document::new_notebook("Test Nested Hierarchy");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    let s_root = make_test_stroke(0, &[(0.0, 100.0), (400.0, 100.0)]);
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_root,
        },
        &mut doc,
    );

    let root_item = doc.pages[0].items[0].clone();
    let id_s1 = history.alloc_item_id();
    let id_s2 = history.alloc_item_id();

    // Slice 1: S -> [S1 (0..180), S2 (220..400)]
    let s1 = make_test_stroke(id_s1, &[(0.0, 100.0), (180.0, 100.0)]);
    let s2 = make_test_stroke(id_s2, &[(220.0, 100.0), (400.0, 100.0)]);
    let item_s1 = make_item(id_s1, root_item.z_index, s1);
    let item_s2 = make_item(id_s2, root_item.z_index, s2);

    history.execute(
        DocCommand::SliceStroke {
            page_id,
            original_stroke: root_item.clone(),
            new_strokes: vec![item_s1.clone(), item_s2.clone()],
        },
        &mut doc,
    );

    assert_eq!(doc.pages[0].items.len(), 2);
    assert_spatial_index_invariant(&doc.pages[0]);

    // Slice 2: S1 -> [S1a (0..80), S1b (100..180)]
    let id_s1a = history.alloc_item_id();
    let id_s1b = history.alloc_item_id();
    let s1a = make_test_stroke(id_s1a, &[(0.0, 100.0), (80.0, 100.0)]);
    let s1b = make_test_stroke(id_s1b, &[(100.0, 100.0), (180.0, 100.0)]);
    let item_s1a = make_item(id_s1a, item_s1.z_index, s1a);
    let item_s1b = make_item(id_s1b, item_s1.z_index, s1b);

    history.execute(
        DocCommand::SliceStroke {
            page_id,
            original_stroke: item_s1.clone(),
            new_strokes: vec![item_s1a.clone(), item_s1b.clone()],
        },
        &mut doc,
    );

    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.items[0].id, id_s1a);
    assert_eq!(page.items[1].id, id_s1b);
    assert_eq!(page.items[2].id, id_s2);
    assert_spatial_index_invariant(page);

    // Undo 1: Reverts S1a, S1b back to S1
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].id, id_s1);
    assert_eq!(page.items[1].id, id_s2);
    assert_spatial_index_invariant(page);

    // Undo 2: Reverts S1, S2 back to root S
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, root_item.id);
    assert_spatial_index_invariant(page);

    // Redo 1: Re-applies root S -> [S1, S2]
    assert!(history.redo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].id, id_s1);
    assert_eq!(page.items[1].id, id_s2);
    assert_spatial_index_invariant(page);

    // Redo 2: Re-applies S1 -> [S1a, S1b]
    assert!(history.redo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.items[0].id, id_s1a);
    assert_eq!(page.items[1].id, id_s1b);
    assert_eq!(page.items[2].id, id_s2);
    assert_spatial_index_invariant(page);
}

#[test]
fn test_slice_stroke_culling_query_fidelity() {
    let mut doc = Document::new_notebook("Test Culling Fidelity");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    // Horizontal stroke from x=0 to x=300 at y=100
    let s_orig = make_test_stroke(0, &[(0.0, 100.0), (300.0, 100.0)]);
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s_orig,
        },
        &mut doc,
    );

    let orig_item = doc.pages[0].items[0].clone();
    let orig_id = orig_item.id;

    // Before slice: query in center (x: 140..160) must find orig
    let hit_before = doc.pages[0]
        .spatial_index
        .locate_in_envelope(140.0, 95.0, 160.0, 105.0);
    assert_eq!(hit_before.len(), 1);
    assert_eq!(hit_before[0].item_id, orig_id);

    // Bisect with gap between x=130 and x=170
    let id_left = history.alloc_item_id();
    let id_right = history.alloc_item_id();
    let s_left = make_test_stroke(id_left, &[(0.0, 100.0), (130.0, 100.0)]);
    let s_right = make_test_stroke(id_right, &[(170.0, 100.0), (300.0, 100.0)]);

    let item_left = make_item(id_left, orig_item.z_index, s_left);
    let item_right = make_item(id_right, orig_item.z_index, s_right);

    history.execute(
        DocCommand::SliceStroke {
            page_id,
            original_stroke: orig_item,
            new_strokes: vec![item_left, item_right],
        },
        &mut doc,
    );

    let page = &doc.pages[0];

    // After slice: center gap has ZERO hits
    let hit_gap = page
        .spatial_index
        .locate_in_envelope(140.0, 95.0, 160.0, 105.0);
    assert!(hit_gap.is_empty(), "Gap should produce zero culling hits");

    // Left region has exactly left fragment
    let hit_left = page
        .spatial_index
        .locate_in_envelope(0.0, 95.0, 50.0, 105.0);
    assert_eq!(hit_left.len(), 1);
    assert_eq!(hit_left[0].item_id, id_left);

    // Right region has exactly right fragment
    let hit_right = page
        .spatial_index
        .locate_in_envelope(200.0, 95.0, 300.0, 105.0);
    assert_eq!(hit_right.len(), 1);
    assert_eq!(hit_right[0].item_id, id_right);

    // Full envelope query returns both fragments in order
    let hit_full = page
        .spatial_index
        .locate_in_envelope(0.0, 95.0, 300.0, 105.0);
    assert_eq!(hit_full.len(), 2);
    assert_eq!(hit_full[0].item_id, id_left);
    assert_eq!(hit_full[1].item_id, id_right);

    // UNDO: query center gap returns orig again
    assert!(history.undo(&mut doc));
    let hit_gap_undo = doc.pages[0]
        .spatial_index
        .locate_in_envelope(140.0, 95.0, 160.0, 105.0);
    assert_eq!(hit_gap_undo.len(), 1);
    assert_eq!(hit_gap_undo[0].item_id, orig_id);

    // REDO: query center gap returns empty
    assert!(history.redo(&mut doc));
    let hit_gap_redo = doc.pages[0]
        .spatial_index
        .locate_in_envelope(140.0, 95.0, 160.0, 105.0);
    assert!(hit_gap_redo.is_empty());
}

#[test]
fn test_slice_stroke_serde_roundtrip() {
    let page_id = Uuid::new_v4();
    let s_orig = make_test_stroke(77, &[(10.0, 10.0), (50.0, 50.0)]);
    let orig_item = make_item(77, 1, s_orig);

    let s1 = make_test_stroke(78, &[(10.0, 10.0), (25.0, 25.0)]);
    let s2 = make_test_stroke(79, &[(35.0, 35.0), (50.0, 50.0)]);
    let new_strokes = vec![make_item(78, 1, s1), make_item(79, 1, s2)];

    let original_cmd = DocCommand::SliceStroke {
        page_id,
        original_stroke: orig_item,
        new_strokes,
    };

    let serialized = serde_json::to_string(&original_cmd).expect("Serialization must succeed");
    let deserialized: DocCommand =
        serde_json::from_str(&serialized).expect("Deserialization must succeed");

    match deserialized {
        DocCommand::SliceStroke {
            page_id: d_page_id,
            original_stroke: d_orig,
            new_strokes: d_new,
        } => {
            assert_eq!(d_page_id, page_id);
            assert_eq!(d_orig.id, 77);
            assert_eq!(d_orig.z_index, 1);
            assert_eq!(d_new.len(), 2);
            assert_eq!(d_new[0].id, 78);
            assert_eq!(d_new[1].id, 79);
        }
        other => panic!("Expected SliceStroke variant, got {:?}", other),
    }
}

#[test]
fn test_page_spatial_index_remove_item_direct() {
    let mut index = PageSpatialIndex::empty();

    let s1 = make_test_stroke(1, &[(0.0, 0.0), (10.0, 10.0)]);
    let item1 = make_item(1, 1, s1);

    let s2 = make_test_stroke(2, &[(20.0, 20.0), (30.0, 30.0)]);
    let item2 = make_item(2, 2, s2);

    index.insert(&item1);
    index.insert(&item2);
    assert_eq!(index.len(), 2);

    // Remove item1: should return true
    assert!(index.remove_item(&item1));
    assert_eq!(index.len(), 1);

    // Removing item1 again: should return false
    assert!(!index.remove_item(&item1));
    assert_eq!(index.len(), 1);

    // Remove item2: should return true
    assert!(index.remove_item(&item2));
    assert_eq!(index.len(), 0);
    assert!(index.is_empty());

    // Degenerate stroke with no points (bounding box None)
    let s_empty = Stroke::new(99, StrokeStyle::default(), PipelineMode::CatmullRomSpline);
    let item_empty = make_item(99, 99, s_empty);
    // Not in tree, remove should return false safely without panic
    assert!(!index.remove_item(&item_empty));
}

/// Simple deterministic pseudo-random number generator for repeatable test runs.
struct DeterministicPrng {
    state: u64,
}

impl DeterministicPrng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }

    fn next_range(&mut self, max: u32) -> u32 {
        if max == 0 {
            0
        } else {
            self.next_u32() % max
        }
    }
}

#[test]
fn test_slice_stroke_spatial_index_lifecycle_1000_cycles() {
    let mut doc = Document::new_notebook("Test 1000 Cycles");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(2000);
    let mut prng = DeterministicPrng::new(0xDEAD_BEEF_CAFE_BABE);

    // Seed document with 15 initial strokes directly in page.items and spatial_index
    // so that undoing history only unwinds SliceStroke commands!
    for i in 1..=15 {
        let x = (i as f64) * 50.0;
        let stroke = make_test_stroke(i, &[(x, 100.0), (x + 30.0, 150.0)]);
        let item = make_item(i, i, stroke);
        doc.pages[0].spatial_index.insert(&item);
        doc.pages[0].items.push(item);
    }
    history.set_next_item_id(16);

    assert_eq!(doc.pages[0].items.len(), 15);
    assert_spatial_index_invariant(&doc.pages[0]);

    const TOTAL_CYCLES: usize = 1_000;

    for cycle in 0..TOTAL_CYCLES {
        let can_undo = history.can_undo();
        let can_redo = history.can_redo();
        let page_has_items = !doc.pages[0].items.is_empty();

        // Decide action:
        // 0: Slice an existing item
        // 1: Undo
        // 2: Redo
        let action = if can_undo && can_redo && page_has_items {
            prng.next_range(3)
        } else if can_undo && page_has_items {
            prng.next_range(2)
        } else if can_undo {
            1
        } else if can_redo {
            2
        } else {
            0
        };

        match action {
            0 => {
                // Execute a new SliceStroke on a randomly chosen item on the page
                let items_len = doc.pages[0].items.len();
                if items_len > 0 {
                    let target_idx = prng.next_range(items_len as u32) as usize;
                    let target_item = doc.pages[0].items[target_idx].clone();

                    // Choose number of resulting fragments: 0 (delete), 1 (trim), 2 (bisect), 3 (multi)
                    let num_frags = prng.next_range(4) as usize;
                    let mut new_strokes = Vec::with_capacity(num_frags);

                    let ItemKind::Stroke(ref target_s) = target_item.kind else {
                        panic!("expected Stroke")
                    };
                    let bbox = target_s
                        .bounding_box()
                        .unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));

                    for f in 0..num_frags {
                        let new_id = history.alloc_item_id();
                        let offset_x = (f as f64) * 5.0;
                        let frag_s = make_test_stroke(
                            new_id,
                            &[
                                (bbox.min_x + offset_x, bbox.min_y),
                                (bbox.min_x + offset_x + 3.0, bbox.min_y + 3.0),
                            ],
                        );
                        new_strokes.push(make_item(new_id, target_item.z_index, frag_s));
                    }

                    history.execute(
                        DocCommand::SliceStroke {
                            page_id,
                            original_stroke: target_item,
                            new_strokes,
                        },
                        &mut doc,
                    );
                }
            }
            1 => {
                // Undo
                let undone = history.undo(&mut doc);
                assert!(undone, "Undo must succeed at cycle {}", cycle);
            }
            2 => {
                // Redo
                let redone = history.redo(&mut doc);
                assert!(redone, "Redo must succeed at cycle {}", cycle);
            }
            _ => unreachable!(),
        }

        // Verify full invariant after EVERY single cycle
        let page = &doc.pages[0];
        assert_spatial_index_invariant(page);
    }

    // Record the post-1000 items state (IDs and count)
    let post_1000_item_ids: Vec<u64> = doc.pages[0].items.iter().map(|it| it.id).collect();

    // Unwind history with Undo until the very beginning, counting undos
    let mut undo_count = 0;
    while history.can_undo() {
        assert!(history.undo(&mut doc));
        undo_count += 1;
        assert_spatial_index_invariant(&doc.pages[0]);
    }

    // Document is back to initial 15 strokes
    assert_eq!(doc.pages[0].items.len(), 15);
    for (idx, item) in doc.pages[0].items.iter().enumerate() {
        assert_eq!(item.id, (idx + 1) as u64);
    }

    // Replay exactly the same number of redos as undos
    for _ in 0..undo_count {
        assert!(history.redo(&mut doc));
        assert_spatial_index_invariant(&doc.pages[0]);
    }

    // Assert that the exact same post-1000 items are restored (set and count equality)
    let replay_set: HashSet<u64> = doc.pages[0].items.iter().map(|it| it.id).collect();
    let post_1000_set: HashSet<u64> = post_1000_item_ids.into_iter().collect();
    assert_eq!(replay_set, post_1000_set);
    assert_eq!(doc.pages[0].items.len(), replay_set.len());

    // Any remaining redos (from cycles that ended while redo stack was non-empty)
    while history.can_redo() {
        assert!(history.redo(&mut doc));
        assert_spatial_index_invariant(&doc.pages[0]);
    }
}

#[test]
fn test_slice_stroke_linear_exact_symmetry_cycles() {
    let mut doc = Document::new_notebook("Test Linear Symmetry");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(500);

    // Initial 10 strokes
    for i in 1..=10 {
        let x = (i as f64) * 40.0;
        let stroke = make_test_stroke(i, &[(x, 50.0), (x + 20.0, 80.0)]);
        history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
    }

    let initial_items = doc.pages[0].items.clone();
    assert_eq!(initial_items.len(), 10);
    assert_spatial_index_invariant(&doc.pages[0]);

    // Apply 30 slice operations of various types
    for i in 0..30 {
        let items_len = doc.pages[0].items.len();
        let target_idx = i % items_len;
        let target_item = doc.pages[0].items[target_idx].clone();

        let slice_type = i % 3; // 0: trim, 1: bisect, 2: 3 fragments
        let num_frags = slice_type + 1;
        let mut new_strokes = Vec::with_capacity(num_frags);

        let ItemKind::Stroke(ref s) = target_item.kind else {
            panic!("expected Stroke")
        };
        let bbox = s.bounding_box().unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));

        for f in 0..num_frags {
            let new_id = history.alloc_item_id();
            let offset_x = (f as f64) * 4.0;
            let frag_s = make_test_stroke(
                new_id,
                &[
                    (bbox.min_x + offset_x, bbox.min_y),
                    (bbox.min_x + offset_x + 2.0, bbox.min_y + 2.0),
                ],
            );
            new_strokes.push(make_item(new_id, target_item.z_index, frag_s));
        }

        history.execute(
            DocCommand::SliceStroke {
                page_id,
                original_stroke: target_item,
                new_strokes,
            },
            &mut doc,
        );
        assert_spatial_index_invariant(&doc.pages[0]);
    }

    let post_slices_items = doc.pages[0].items.clone();
    let num_slices = 30;

    // Perform 10 full cycles of Undo 30 / Redo 30
    for cycle in 0..10 {
        // Undo all 30 slice commands
        for _ in 0..num_slices {
            assert!(history.undo(&mut doc), "Undo failed at cycle {}", cycle);
            assert_spatial_index_invariant(&doc.pages[0]);
        }

        // Verify exact array equality with initial_items
        assert_eq!(
            doc.pages[0].items.len(),
            initial_items.len(),
            "Length mismatch at cycle {}",
            cycle
        );
        for (it1, it2) in doc.pages[0].items.iter().zip(initial_items.iter()) {
            assert_eq!(it1.id, it2.id);
            assert_eq!(it1.z_index, it2.z_index);
        }

        // Redo all 30 slice commands
        for _ in 0..num_slices {
            assert!(history.redo(&mut doc), "Redo failed at cycle {}", cycle);
            assert_spatial_index_invariant(&doc.pages[0]);
        }

        // Verify exact array equality with post_slices_items
        assert_eq!(
            doc.pages[0].items.len(),
            post_slices_items.len(),
            "Length mismatch after redo at cycle {}",
            cycle
        );
        for (it1, it2) in doc.pages[0].items.iter().zip(post_slices_items.iter()) {
            assert_eq!(it1.id, it2.id);
            assert_eq!(it1.z_index, it2.z_index);
        }
    }
}
