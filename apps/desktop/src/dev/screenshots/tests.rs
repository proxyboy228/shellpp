//! Tests of the screenshot mode. The state tests drive the same code as a
//! real run ([`reach_state`], [`check_state`]) in GPUI's headless test
//! platform, so a state that cannot be reached fails here and not only in a
//! launch of the real app (which renders offscreen and is limited per
//! iteration). Profiles are temporary, secrets stay in memory.

use super::*;
use crate::hosts::host_card_id;
use crate::testing::TestProfile;
use gpui::{Bounds, TestAppContext, WindowOptions};

#[test]
fn sizes_parse_and_validate() {
    assert_eq!(
        WindowSize::parse("1920x1200"),
        Some(WindowSize::new(1920, 1200))
    );
    assert_eq!(WindowSize::parse(" 1280X800 "), Some(MAIN_SIZE));
    for bad in [
        "",
        "1920",
        "x800",
        "1920x",
        "axb",
        "100x100",
        "1920x99999",
        "-1x800",
    ] {
        assert_eq!(WindowSize::parse(bad), None, "{bad}");
        assert_eq!(ShotSize::parse(bad), None, "{bad}");
    }
    assert_eq!(ShotSize::parse("1280x800"), Some(DEFAULT_SIZE));
    assert_eq!(
        ShotSize::parse("1920x1200, 1280x800,960x600"),
        Some(FULL_SET)
    );
    assert_eq!(FULL_SET.to_string(), "1920x1200,1280x800,960x600");
    // Duplicates, empty parts and too many sizes are refused.
    assert_eq!(ShotSize::parse("1280x800,1280x800"), None);
    assert_eq!(ShotSize::parse("1280x800,"), None);
    let many: Vec<String> = (0..9).map(|n| format!("{}x800", 1000 + n)).collect();
    assert_eq!(ShotSize::parse(&many.join(",")), None);
}

/// The minimum size of the screenshots is the app's minimum window size
/// (lead decision 4: 960x600).
#[test]
fn the_minimum_size_is_the_apps_minimum() {
    assert_eq!(MIN_SIZE, WindowSize::new(960, 600));
    assert_eq!(
        (MIN_SIZE.width as f32, MIN_SIZE.height as f32),
        (app::MIN_WINDOW_WIDTH, app::MIN_WINDOW_HEIGHT)
    );
    assert_eq!(
        (MAIN_SIZE.width as f32, MAIN_SIZE.height as f32),
        (app::DEFAULT_WINDOW_WIDTH, app::DEFAULT_WINDOW_HEIGHT)
    );
}

/// The full set keeps the original 12 files and adds every state of the
/// owner's list at 1280x800 (and the "+N" chips) and the first run at the
/// minimum size, in both themes.
#[test]
fn the_full_set_covers_every_state_in_both_themes() {
    let files = expected_files(&FULL_SET);
    for state in ["root", "group", "host-panel"] {
        for size in ["1920x1200", "1280x800"] {
            for theme in ["dark", "light"] {
                let name = format!("{size}-{theme}-{state}.png");
                assert!(files.contains(&name), "{name}");
            }
        }
    }
    for state in [
        "context-menu",
        "search-address",
        "search-results",
        "search-no-results",
        "three-selected",
        "empty",
        "list-view",
        "new-host",
        "quick-connect-toast",
        "tag-filter",
        "tag-chips",
        "show-more",
        "first-run",
        "unlock",
    ] {
        for theme in ["dark", "light"] {
            let name = format!("1280x800-{theme}-{state}.png");
            assert!(files.contains(&name), "{name}");
        }
    }
    for state in ["root", "host-panel", "first-run"] {
        for theme in ["dark", "light"] {
            let name = format!("960x600-{theme}-{state}.png");
            assert!(files.contains(&name), "{name}");
        }
    }
    assert_eq!(files.len(), 6 + 34 + 6);
    let unique: std::collections::BTreeSet<_> = files.iter().collect();
    assert_eq!(unique.len(), files.len(), "no file is written twice");
    // One size: the original three states.
    assert_eq!(
        expected_files(&ShotSize::parse("1600x900").unwrap()),
        [
            "1600x900-dark-root.png",
            "1600x900-light-root.png",
            "1600x900-dark-group.png",
            "1600x900-light-group.png",
            "1600x900-dark-host-panel.png",
            "1600x900-light-host-panel.png",
        ]
    );
}

#[test]
fn host_cards_are_found_by_their_id_prefix() {
    let id = Uuid::now_v7();
    assert_eq!(host_card_id(id).as_ref(), format!("{HOST_CARD_PREFIX}{id}"));
}

/// The run's data lives inside `SHELLPP_DATA_DIR` when it is set.
#[test]
fn demo_profiles_can_live_in_a_given_folder() {
    let parent = TempProfile::create("shellpp-test-shots").unwrap();
    let profile = demo::create_profile_in(parent.path()).unwrap();
    assert!(profile.path().starts_with(parent.path()));
    let path = profile.path().to_path_buf();
    drop(profile);
    assert!(!path.exists());
}

fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(Appearance::Dark, cx);
        app::root::bind_keys(cx);
    });
}

fn test_window(
    state: &Entity<AppState>,
    size: WindowSize,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<RootView>) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            size.logical(),
        ))),
        ..Default::default()
    };
    cx.update(|cx| app::open_window_with(state.clone(), options, cx))
        .expect("test window")
}

/// Every demo state of the full set is reached from the reset state and
/// passes its check, at 1280x800 and at the minimum size; the reset brings
/// the window back to the plain root each time. The demo data (300 hosts,
/// 5 groups) is seeded into a temporary test profile.
#[gpui_kit::test]
async fn every_demo_state_is_reached(cx: &mut TestAppContext) {
    setup(cx);
    let opened = TestProfile::new().into_unlocked();
    let summary = demo::seed(&opened.core).expect("demo data");
    let state = cx.new(|cx| AppState::new(opened, cx));
    let first_group = summary.groups.first().copied();

    for size in [MAIN_SIZE, MIN_SIZE] {
        let (handle, root) = test_window(&state, size, cx);
        let mut async_cx = cx.to_async();
        let demo_states = states_for(size)
            .iter()
            .copied()
            .filter(|state| state.scene() == Scene::Demo);
        let mut failures = Vec::new();
        for shot_state in demo_states {
            let result = async {
                reach_state(
                    &mut async_cx,
                    handle,
                    &root,
                    &state,
                    first_group,
                    shot_state,
                )
                .await?;
                settle(&mut async_cx, handle).await?;
                check_state(&mut async_cx, handle, &root, shot_state)
            }
            .await;
            if let Err(error) = result {
                failures.push(format!("{size}: {error}"));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
        // The reset returns to the plain root.
        reset_demo(&mut async_cx, handle, &root, &state)
            .await
            .expect("reset");
        check_state(&mut async_cx, handle, &root, ShotState::Root).expect("root after reset");
        cx.update(|cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        });
    }
    // Settings written by the states (the list view) were put back.
    cx.update(|cx| {
        assert_eq!(state.read(cx).hosts_layout(), HostsLayout::Tiles);
    });
    state.update(cx, |state, _| state.shutdown());
}

/// The first-run, unlock and empty screens are shown on fresh empty
/// profiles, which are deleted afterwards.
#[gpui_kit::test]
async fn fresh_profile_states_show_their_screens(cx: &mut TestAppContext) {
    setup(cx);
    let parent = TempProfile::create("shellpp-test-shots").unwrap();
    for shot_state in [ShotState::FirstRun, ShotState::Empty, ShotState::Unlock] {
        let Scene::Fresh(stage) = shot_state.scene() else {
            panic!("{shot_state:?} is a fresh-profile state");
        };
        let mut async_cx = cx.to_async();
        let fresh = open_fresh_state(&mut async_cx, parent.path(), stage).expect("fresh profile");
        let (handle, root) = test_window(&fresh, MAIN_SIZE, cx);
        settle(&mut async_cx, handle).await.expect("settle");
        check_state(&mut async_cx, handle, &root, shot_state)
            .unwrap_or_else(|error| panic!("{error}"));
        cx.update(|cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        });
        cx.run_until_parked();
        fresh.update(cx, |fresh, _| fresh.shutdown());
    }
    let left = std::fs::read_dir(parent.path()).unwrap().count();
    assert_eq!(left, 0, "the fresh profiles were removed");
}
