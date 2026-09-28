use super::*;
use std::fs::{self, File, FileTimes};
use std::io::Write;

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "syspulse-cleanup-test-{name}-{}-{}",
        std::process::id(),
        NEXT_SCAN_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn only_safe_categories_are_selected_by_default() {
    assert!(cleanup_policy("win-temp").default_selected);
    assert!(cleanup_policy("installer-cache").default_selected);
    for id in [
        "browser-cache",
        "app-cache",
        "webview-cache",
        "rust-target",
        "shader-cache",
    ] {
        assert!(!cleanup_policy(id).default_selected, "{id}");
    }
}

#[test]
fn age_filter_excludes_recent_files() {
    let root = temp_root("age");
    fs::create_dir_all(&root).unwrap();
    let recent = root.join("recent.tmp");
    let old = root.join("old.tmp");
    fs::write(&recent, vec![1u8; 11]).unwrap();
    let mut file = File::create(&old).unwrap();
    file.write_all(&[2u8; 17]).unwrap();
    file.set_times(
        FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60)),
    )
    .unwrap();

    let expected = file_details(&old, &fs::metadata(&old).unwrap())
        .map(|value| value.2)
        .unwrap_or(17);
    assert_eq!(dir_size_with_min_age(&root, Some(7)), (expected, 1));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_root_must_be_a_real_directory() {
    let root = temp_root("root");
    fs::write(&root, b"not a directory").unwrap();
    assert!(safe_cleanup_root(&root).is_none());
    fs::remove_file(root).unwrap();
}

#[test]
fn caution_category_requires_explicit_confirmation() {
    assert!(validate_risk_confirmation([CleanupRisk::Caution], false, false).is_err());
    assert!(validate_risk_confirmation([CleanupRisk::Caution], true, false).is_ok());
}

#[test]
fn advanced_category_requires_explicit_confirmation() {
    assert!(validate_risk_confirmation([CleanupRisk::Advanced], true, false).is_err());
    assert!(validate_risk_confirmation([CleanupRisk::Advanced], false, true).is_ok());
}

#[test]
fn cancelled_or_incomplete_scan_is_never_cleanable() {
    let base = ScanResult {
        scan_id: "scan-1".into(),
        categories: Vec::new(),
        total_size_bytes: 0,
        total_file_count: 0,
        scanned_at_ms: 0,
        expires_at_ms: 0,
        duration_ms: 0,
        scanned_paths: 0,
        skipped_paths: 0,
        ignored_paths: 0,
        hotspot_count: 0,
        scout_workers: 1,
        engineer_workers: 2,
        scout_tasks: 0,
        engineer_tasks: 0,
        project_roots: Vec::new(),
        warnings: Vec::new(),
        complete: true,
        cancelled: false,
    };
    assert!(scan_is_cleanable(&base));
    assert!(!scan_is_cleanable(&ScanResult {
        cancelled: true,
        ..base.clone()
    }));
    assert!(!scan_is_cleanable(&ScanResult {
        complete: false,
        ..base.clone()
    }));
    assert!(!scan_is_cleanable(&ScanResult {
        scan_id: String::new(),
        ..base
    }));
}

#[test]
fn path_normalization_handles_extended_windows_prefixes() {
    assert_eq!(normalize_path_key(r"\\?\C:\Temp\"), r"c:\temp");
    assert_eq!(
        normalize_path_key(r"\\?\UNC\server\share\Cache"),
        r"\\server\share\cache"
    );
    assert!(paths_overlap_keys(r"c:\work", r"c:\work\build"));
    assert!(!paths_overlap_keys(r"c:\work", r"c:\workspace"));
}

#[test]
fn scan_result_contains_only_supplied_categories() {
    let diagnostics = ScanDiagnostics::default();
    let categories = vec![CleanupCategory {
        id: "win-temp".into(),
        name: "Windows 临时文件".into(),
        description: "test".into(),
        size_bytes: 0,
        file_count: 0,
        paths: Vec::new(),
        risk_level: CleanupRisk::Safe,
        default_selected: true,
        min_age_days: Some(7),
    }];
    let result = finalize_scan_result(
        categories,
        &diagnostics,
        ColonyConfig {
            scout_workers: 1,
            engineer_workers: 2,
        },
        &std::collections::HashSet::new(),
    );
    assert_eq!(result.categories.len(), 1);
    assert_eq!(result.categories[0].id, "win-temp");
}

#[test]
fn hotspot_paths_prioritize_recent_entries_and_prune_missing_paths() {
    let root = temp_root("hotspot-order");
    let older = root.join("older");
    let newer = root.join("newer");
    fs::create_dir_all(&older).unwrap();
    fs::create_dir_all(&newer).unwrap();
    let index = HotspotIndex {
        schema_version: HOTSPOT_SCHEMA_VERSION,
        entries: vec![
            HotspotEntry {
                path: older.to_string_lossy().to_string(),
                category_id: "rust-target".into(),
                matched_rule: "old".into(),
                last_seen_ms: 1,
                last_scanned_ms: 1,
                size_bytes: 1,
                file_count: 1,
                miss_count: 0,
            },
            HotspotEntry {
                path: newer.to_string_lossy().to_string(),
                category_id: "cpp-cache".into(),
                matched_rule: "new".into(),
                last_seen_ms: 2,
                last_scanned_ms: 2,
                size_bytes: 2,
                file_count: 2,
                miss_count: 0,
            },
            HotspotEntry {
                path: root.join("missing").to_string_lossy().to_string(),
                category_id: "python-cache".into(),
                matched_rule: "missing".into(),
                last_seen_ms: 3,
                last_scanned_ms: 3,
                size_bytes: 3,
                file_count: 3,
                miss_count: 0,
            },
        ],
    };
    let paths = hotspot_paths(&index);
    assert_eq!(paths, vec![newer, older]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hotspot_index_round_trips_and_corrupt_json_falls_back_to_default() {
    let root = temp_root("hotspot-json");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("hotspots.json");
    let index = HotspotIndex {
        schema_version: HOTSPOT_SCHEMA_VERSION,
        entries: vec![HotspotEntry {
            path: root.to_string_lossy().to_string(),
            category_id: "rust-target".into(),
            matched_rule: "Cargo.toml + target".into(),
            last_seen_ms: 10,
            last_scanned_ms: 11,
            size_bytes: 12,
            file_count: 13,
            miss_count: 0,
        }],
    };
    write_json_atomic(&path, &index).unwrap();
    let decoded: HotspotIndex = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(decoded.entries.len(), 1);
    fs::write(&path, b"not-json").unwrap();
    let decoded = serde_json::from_slice::<HotspotIndex>(&fs::read(&path).unwrap())
        .ok()
        .filter(|value| value.schema_version == HOTSPOT_SCHEMA_VERSION)
        .unwrap_or_default();
    assert!(decoded.entries.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn export_document_records_category_and_path_selection() {
    let selected_path = PathDetail {
        path: r"C:\cache\selected".into(),
        size_bytes: 10,
        file_count: 1,
        matched_rule: "rule".into(),
        source: "tool-global-cache".into(),
        volume_serial: Some(1),
        file_id: Some(1),
    };
    let excluded_path = PathDetail {
        path: r"C:\cache\excluded".into(),
        size_bytes: 20,
        file_count: 2,
        matched_rule: "rule".into(),
        source: "tool-global-cache".into(),
        volume_serial: Some(1),
        file_id: Some(2),
    };
    let scan = ScanResult {
        scan_id: "scan-export".into(),
        categories: vec![CleanupCategory {
            id: "node-cache".into(),
            name: "Node.js 缓存".into(),
            description: "test".into(),
            size_bytes: 30,
            file_count: 3,
            paths: vec![selected_path.clone(), excluded_path.clone()],
            risk_level: CleanupRisk::Advanced,
            default_selected: false,
            min_age_days: None,
        }],
        total_size_bytes: 30,
        total_file_count: 3,
        scanned_at_ms: 100,
        expires_at_ms: 200,
        duration_ms: 5,
        scanned_paths: 2,
        skipped_paths: 0,
        ignored_paths: 0,
        hotspot_count: 1,
        scout_workers: 1,
        engineer_workers: 2,
        scout_tasks: 2,
        engineer_tasks: 2,
        project_roots: vec![r"C:\work".into()],
        warnings: Vec::new(),
        complete: true,
        cancelled: false,
    };
    let document =
        build_cleanup_export_document(scan, vec!["node-cache".into()], vec![excluded_path.path]);
    assert_eq!(document.groups.len(), 1);
    assert_eq!(document.groups[0].paths.len(), 1);
    assert_eq!(document.groups[0].paths[0].path, selected_path.path);
    assert_eq!(document.project_roots, vec![r"C:\work"]);
    assert_eq!(document.schema_version, 3);
    assert_eq!(document.selected_total_size_bytes, 10);
    assert_eq!(document.selected_total_file_count, 1);
    assert_eq!(document.groups[0].paths[0].volume_serial, Some(1));
    assert_eq!(document.groups[0].paths[0].file_id, Some(1));
}

#[test]
fn export_document_omits_unselected_categories() {
    let scan = ScanResult {
        scan_id: "scan-export-filter".into(),
        categories: vec![CleanupCategory {
            id: "rust-target".into(),
            name: "Rust 编译缓存".into(),
            description: "test".into(),
            size_bytes: 42,
            file_count: 2,
            paths: vec![PathDetail {
                path: r"C:\work\target".into(),
                size_bytes: 42,
                file_count: 2,
                matched_rule: "Cargo target".into(),
                source: "project-discovery".into(),
                volume_serial: Some(1),
                file_id: Some(2),
            }],
            risk_level: CleanupRisk::Advanced,
            default_selected: false,
            min_age_days: None,
        }],
        total_size_bytes: 42,
        total_file_count: 2,
        scanned_at_ms: 100,
        expires_at_ms: 200,
        duration_ms: 5,
        scanned_paths: 2,
        skipped_paths: 0,
        ignored_paths: 0,
        hotspot_count: 0,
        scout_workers: 2,
        engineer_workers: 2,
        scout_tasks: 12,
        engineer_tasks: 1,
        project_roots: Vec::new(),
        warnings: Vec::new(),
        complete: true,
        cancelled: false,
    };
    let document = build_cleanup_export_document(scan, Vec::new(), Vec::new());
    assert!(document.groups.is_empty());
    assert_eq!(document.selected_total_size_bytes, 0);
}

#[test]
fn duplicate_paths_are_owned_by_the_first_category_once() {
    let shared = PathDetail {
        path: r"C:\cache\shared".into(),
        size_bytes: 10,
        file_count: 2,
        matched_rule: "shared".into(),
        source: "tool-global-cache".into(),
        volume_serial: Some(7),
        file_id: Some(9),
    };
    let mut categories = vec![
        CleanupCategory {
            id: "node-cache".into(),
            name: "Node".into(),
            description: String::new(),
            size_bytes: 10,
            file_count: 2,
            paths: vec![shared.clone()],
            risk_level: CleanupRisk::Advanced,
            default_selected: false,
            min_age_days: None,
        },
        CleanupCategory {
            id: "app-cache".into(),
            name: "App".into(),
            description: String::new(),
            size_bytes: 10,
            file_count: 2,
            paths: vec![PathDetail {
                path: r"\\?\C:\CACHE\SHARED".into(),
                ..shared
            }],
            risk_level: CleanupRisk::Caution,
            default_selected: false,
            min_age_days: None,
        },
    ];

    deduplicate_category_paths(&mut categories);

    assert_eq!(categories[0].paths.len(), 1);
    assert!(categories[1].paths.is_empty());
    assert_eq!(categories[1].size_bytes, 0);
    assert_eq!(categories[1].file_count, 0);
}

#[test]
fn programming_cache_processes_block_conservatively() {
    let processes = vec![RunningProcessInfo {
        name: "cargo.exe".into(),
        stem: "cargo".into(),
        command_paths: vec![normalize_path_key(r"C:\unrelated\workspace")],
        cwd: Some(normalize_path_key(r"C:\unrelated\workspace")),
    }];
    assert_eq!(
        running_process_blockers_for_path("rust-target", r"D:\project\target", &processes),
        vec!["cargo.exe"]
    );
}

#[test]
fn programming_process_lists_cover_supported_toolchains() {
    let rust = watched_processes("rust-target").unwrap().0;
    assert!(rust.contains(&"cargo") && rust.contains(&"rustc"));

    let cpp = watched_processes("cpp-cache").unwrap().0;
    for process in [
        "cmake", "ninja", "msbuild", "cl", "clang", "gcc", "ccache", "sccache",
    ] {
        assert!(cpp.contains(&process), "{process}");
    }

    let python = watched_processes("python-cache").unwrap().0;
    for process in ["pip", "uv", "poetry", "pdm", "pytest", "mypy", "ruff"] {
        assert!(python.contains(&process), "{process}");
    }

    assert!(watched_processes("go-cache").unwrap().0.contains(&"go"));
    assert!(watched_processes("dotnet-cache")
        .unwrap()
        .0
        .contains(&"nuget"));
    assert!(watched_processes("java-cache")
        .unwrap()
        .0
        .contains(&"gradle"));
}

#[test]
fn ordinary_access_warning_does_not_make_scan_incomplete() {
    let diagnostics = ScanDiagnostics::default();
    diagnostics.warn("C:\\protected: access denied");
    let result = finalize_scan_result(
        Vec::new(),
        &diagnostics,
        ColonyConfig {
            scout_workers: 1,
            engineer_workers: 2,
        },
        &std::collections::HashSet::new(),
    );
    assert!(result.complete);
    assert_eq!(result.skipped_paths, 1);
}

#[test]
fn budget_truncation_makes_scan_incomplete() {
    let diagnostics = ScanDiagnostics::default();
    diagnostics.truncated.store(true, Ordering::Relaxed);
    let result = finalize_scan_result(
        Vec::new(),
        &diagnostics,
        ColonyConfig {
            scout_workers: 1,
            engineer_workers: 2,
        },
        &std::collections::HashSet::new(),
    );
    assert!(!result.complete);
}

#[test]
fn application_processes_only_block_overlapping_cache_paths() {
    let processes = vec![RunningProcessInfo {
        name: "code.exe".into(),
        stem: "code".into(),
        command_paths: vec![normalize_path_key(r"C:\work\project")],
        cwd: Some(normalize_path_key(r"C:\work\project")),
    }];
    assert!(running_process_blockers_for_path(
        "app-cache",
        r"C:\Users\tester\AppData\Roaming\Code\Cache",
        &processes
    )
    .is_empty());

    let overlapping = vec![RunningProcessInfo {
        name: "code.exe".into(),
        stem: "code".into(),
        command_paths: Vec::new(),
        cwd: Some(normalize_path_key(
            r"C:\Users\tester\AppData\Roaming\Code\Cache",
        )),
    }];
    assert_eq!(
        running_process_blockers_for_path(
            "app-cache",
            r"C:\Users\tester\AppData\Roaming\Code\Cache",
            &overlapping
        ),
        vec!["code.exe"]
    );
}

#[test]
fn process_argument_paths_extract_equals_values() {
    let keys = process_argument_path_keys(
        r#"--user-data-dir=\\?\C:\Users\tester\AppData\Local\Browser Data"#,
    );
    assert!(keys.contains(&normalize_path_key(
        r"C:\Users\tester\AppData\Local\Browser Data"
    )));
}

#[test]
fn partial_path_progress_only_fills_unprocessed_items() {
    assert_eq!(remaining_path_items(10, 13, 8), 5);
    assert_eq!(remaining_path_items(10, 25, 8), 0);
    assert_eq!(remaining_path_items(10, 9, 8), 8);
}

#[test]
fn cleanup_root_identity_rejects_replaced_directory() {
    let root = temp_root("identity");
    fs::create_dir_all(&root).unwrap();
    let detail = PathDetail {
        path: root.to_string_lossy().to_string(),
        size_bytes: 0,
        file_count: 0,
        ..path_detail_identity(&root)
    };
    assert!(revalidate_cleanup_root(&detail).is_some());

    let old = root.with_extension("old");
    fs::rename(&root, &old).unwrap();
    fs::create_dir_all(&root).unwrap();
    assert!(revalidate_cleanup_root(&detail).is_none());

    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(old).unwrap();
}

#[test]
fn cpp_build_directory_requires_markers_and_artifacts() {
    let root = temp_root("cpp");
    let build = root.join("build");
    fs::create_dir_all(build.join("CMakeFiles")).unwrap();
    fs::write(build.join("CMakeCache.txt"), b"cache").unwrap();
    assert!(is_verified_cpp_build_dir(&build, "build"));

    let fake = root.join("fake").join("build");
    fs::create_dir_all(&fake).unwrap();
    assert!(!is_verified_cpp_build_dir(&fake, "build"));

    let msbuild_root = root.join("msbuild");
    let obj = msbuild_root.join("obj");
    fs::create_dir_all(&obj).unwrap();
    fs::write(msbuild_root.join("sample.vcxproj"), b"project").unwrap();
    fs::write(obj.join("sample.pdb"), b"symbols").unwrap();
    assert!(is_verified_cpp_build_dir(&obj, "obj"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn python_project_cache_requires_project_marker() {
    let root = temp_root("python");
    let cache = root.join(".mypy_cache");
    fs::create_dir_all(&cache).unwrap();
    assert!(!is_verified_python_project_cache(&cache, ".mypy_cache"));
    fs::write(root.join("pyproject.toml"), b"[project]").unwrap();
    assert!(is_verified_python_project_cache(&cache, ".mypy_cache"));
    assert!(!is_verified_python_project_cache(
        &root.join(".venv"),
        ".venv"
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_discovery_skips_dangerous_and_duplicate_roots() {
    let root = temp_root("project-skip");
    let appdata = root.join("AppData");
    let code = root.join("Code");
    fs::create_dir_all(&appdata).unwrap();
    fs::create_dir_all(&code).unwrap();
    assert!(should_skip_project_path(&root, &appdata, "appdata", 0));
    assert!(!should_skip_project_path(&root, &code, "code", 0));

    let mut paths = vec![
        code.clone(),
        PathBuf::from(code.to_string_lossy().to_uppercase()),
    ];
    deduplicate_paths(&mut paths);
    assert_eq!(paths.len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_policy_rejects_files_modified_after_scan() {
    let root = temp_root("post-scan");
    fs::create_dir_all(&root).unwrap();
    let file_path = root.join("new.tmp");
    fs::write(&file_path, b"new").unwrap();
    let metadata = fs::metadata(&file_path).unwrap();
    let policy = CleanupFilePolicy {
        min_age_days: None,
        scanned_at: Some(SystemTime::UNIX_EPOCH),
    };
    assert_eq!(
        policy.file_decision(&metadata),
        CleanupPolicyDecision::ChangedAfterScan
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_policy_uses_scan_time_for_age_eligibility() {
    let root = temp_root("age-at-scan");
    fs::create_dir_all(&root).unwrap();
    let file_path = root.join("became-old.tmp");
    fs::write(&file_path, b"cache").unwrap();
    let scanned_at = SystemTime::now() + Duration::from_secs(1);
    let modified = scanned_at - Duration::from_secs(6 * 24 * 60 * 60);
    File::options()
        .write(true)
        .open(&file_path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    let policy = CleanupFilePolicy {
        min_age_days: Some(7),
        scanned_at: Some(scanned_at),
    };
    assert_eq!(
        policy.file_decision(&fs::metadata(&file_path).unwrap()),
        CleanupPolicyDecision::NotEligibleAtScan
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_policy_keeps_age_filter_at_delete_time() {
    let root = temp_root("delete-age");
    fs::create_dir_all(&root).unwrap();
    let file_path = root.join("recent.tmp");
    fs::write(&file_path, b"recent").unwrap();
    let metadata = fs::metadata(&file_path).unwrap();
    let policy = CleanupFilePolicy {
        min_age_days: Some(7),
        scanned_at: Some(SystemTime::now() + Duration::from_secs(1)),
    };
    assert_eq!(
        policy.file_decision(&metadata),
        CleanupPolicyDecision::NotEligibleAtScan
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn custom_project_roots_are_canonicalized_and_deduplicated() {
    let root = temp_root("custom-root");
    fs::create_dir_all(&root).unwrap();
    let values = vec![
        root.to_string_lossy().to_string(),
        root.to_string_lossy().to_uppercase(),
        temp_root("missing").to_string_lossy().to_string(),
    ];
    let normalized = normalize_custom_project_roots(values);
    assert_eq!(normalized.len(), 1);
    assert!(normalized[0].is_dir());
    assert_eq!(
        directory_identity(&normalized[0]),
        directory_identity(&root)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn custom_project_roots_are_scanned_before_broad_roots() {
    let root = temp_root("custom-priority");
    fs::create_dir_all(&root).unwrap();
    let roots = project_search_roots(std::slice::from_ref(&root));
    assert!(roots.first().is_some_and(|first| same_path(first, &root)));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_fixed_volume_is_added_as_a_shallow_probe() {
    let fixed_a = temp_root("fixed-plan-a");
    let fixed_b = temp_root("fixed-plan-b");
    fs::create_dir_all(&fixed_a).unwrap();
    fs::create_dir_all(&fixed_b).unwrap();

    let plan = project_search_plan(&[], &[fixed_a.clone(), fixed_b.clone()]);
    for fixed in [&fixed_a, &fixed_b] {
        assert!(plan.iter().any(|root| {
            root.mode == ProjectScoutMode::FixedVolumeProbe && same_path(&root.path, fixed)
        }));
    }

    fs::remove_dir_all(fixed_a).unwrap();
    fs::remove_dir_all(fixed_b).unwrap();
}

#[test]
fn deep_roots_take_precedence_over_duplicate_volume_probe_roots() {
    let root = temp_root("fixed-plan-priority");
    fs::create_dir_all(&root).unwrap();

    let plan = project_search_plan(std::slice::from_ref(&root), std::slice::from_ref(&root));
    let matches = plan
        .iter()
        .filter(|candidate| same_path(&candidate.path, &root))
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].mode, ProjectScoutMode::Deep);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn duplicate_mount_points_are_deduplicated_by_directory_identity() {
    let root = temp_root("mount-dedup");
    fs::create_dir_all(&root).unwrap();
    let alias = PathBuf::from(root.to_string_lossy().to_uppercase());
    let mut roots = vec![root.clone(), alias];

    deduplicate_directory_roots(&mut roots);
    assert_eq!(roots.len(), 1);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fixed_volume_probe_promotes_marked_project_to_deep_scan() {
    let volume = temp_root("fixed-probe");
    let project = volume.join("misc").join("sample");
    let target = project.join("target");
    fs::create_dir_all(target.join("debug")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        b"[package]\nname='sample'\nversion='0.1.0'\n",
    )
    .unwrap();

    let plan = vec![ProjectSearchRoot {
        path: volume.clone(),
        mode: ProjectScoutMode::FixedVolumeProbe,
    }];
    let roots = vec![volume.clone()];
    let queue = ProjectScoutQueue::from_plan(&plan);
    let diagnostics = Arc::new(ScanDiagnostics::default());
    let _guard = ActiveScanDiagnostics::install(diagnostics);
    let found = AtomicUsize::new(0);
    let volume_probe_tasks = AtomicUsize::new(1);
    let volume_probe_limit_reported = AtomicBool::new(false);
    let mut discovery = ProjectCacheDiscovery::default();

    while let Some(task) = queue.pop() {
        scan_project_cache_task(
            task,
            &roots,
            &queue,
            &found,
            &volume_probe_tasks,
            &volume_probe_limit_reported,
            &mut discovery,
        );
        queue.complete();
    }
    assert!(discovery.rust.iter().any(|path| same_path(path, &target)));

    fs::remove_dir_all(volume).unwrap();
}

#[test]
fn nested_project_roots_are_assigned_once() {
    let root = temp_root("assigned-root");
    let child = root.join("projects");
    fs::create_dir_all(&child).unwrap();
    let roots = vec![root.clone(), child.clone()];

    assert!(is_separately_assigned_root(&root, &child, &roots));
    assert!(!is_separately_assigned_root(&child, &child, &roots));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scout_queue_does_not_finish_while_a_worker_can_enqueue_children() {
    let root = temp_root("scout-queue-race");
    let queue = Arc::new(ProjectScoutQueue::new(std::slice::from_ref(&root)));
    let task = queue.pop().unwrap();
    let waiting = queue.clone();
    let waiter = std::thread::spawn(move || waiting.pop());
    std::thread::sleep(Duration::from_millis(10));
    let child = ProjectScoutTask {
        root: root.clone(),
        dir: root.join("child"),
        depth: 1,
        mode: ProjectScoutMode::Deep,
    };
    queue.push(child.clone());
    queue.complete();
    let received = waiter.join().unwrap().unwrap();
    assert!(same_path(&received.dir, &child.dir));
    queue.complete();
    let _ = task;
}

#[test]
fn directory_measure_cache_reuses_result_without_zeroing_later_categories() {
    let root = temp_root("measure-cache");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("cache.bin"), vec![7u8; 64]).unwrap();
    let cache = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let _cache_guard = ActiveScanDirectoryMeasures::install(cache.clone());

    let first = dir_size(&root);
    let second = dir_size(&root);

    assert_eq!(first, second);
    assert_eq!(first.1, 1);
    assert!(first.0 > 0);
    assert_eq!(cache.lock().unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scanner_handles_many_small_files_and_deep_directories() {
    let root = temp_root("performance");
    let mut leaf = root.clone();
    for depth in 0..20 {
        leaf = leaf.join(format!("level-{depth}"));
    }
    fs::create_dir_all(&leaf).unwrap();
    for index in 0..2_000 {
        fs::write(leaf.join(format!("file-{index}.tmp")), [index as u8]).unwrap();
    }

    let started = Instant::now();
    let (_size, count) = dir_size(&root);
    assert_eq!(count, 2_000);
    assert!(started.elapsed() < Duration::from_secs(10));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sandbox_cleanup_deletes_only_scanned_files() {
    let root = temp_root("sandbox-clean");
    fs::create_dir_all(&root).unwrap();
    let scanned = root.join("scanned.tmp");
    let newer = root.join("newer.tmp");
    fs::write(&scanned, b"scanned").unwrap();
    fs::write(&newer, b"newer").unwrap();

    let scanned_time = SystemTime::now() - Duration::from_secs(30);
    let future_time = SystemTime::now() + Duration::from_secs(30);
    File::options()
        .write(true)
        .open(&scanned)
        .unwrap()
        .set_times(FileTimes::new().set_modified(scanned_time))
        .unwrap();
    File::options()
        .write(true)
        .open(&newer)
        .unwrap()
        .set_times(FileTimes::new().set_modified(future_time))
        .unwrap();

    let policy = CleanupFilePolicy {
        min_age_days: None,
        scanned_at: Some(SystemTime::now()),
    };
    let mut deleted = 0;
    for entry in fs::read_dir(&root).unwrap().flatten() {
        let metadata = entry.metadata().unwrap();
        if policy.file_decision(&metadata) == CleanupPolicyDecision::Allow {
            fs::remove_file(entry.path()).unwrap();
            deleted += 1;
        }
    }
    assert_eq!(deleted, 1);
    assert!(!scanned.exists());
    assert!(newer.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_policy_rejects_directories_created_after_scan() {
    let root = temp_root("new-directory");
    fs::create_dir_all(&root).unwrap();
    let policy = CleanupFilePolicy {
        min_age_days: None,
        scanned_at: Some(SystemTime::UNIX_EPOCH),
    };
    assert_eq!(
        policy.directory_decision(&fs::metadata(&root).unwrap()),
        CleanupPolicyDecision::ChangedAfterScan
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn cleanup_policy_rejects_new_file_with_backdated_modified_time() {
    let root = temp_root("backdated-new-file");
    fs::create_dir_all(&root).unwrap();
    let scanned_at = SystemTime::now() - Duration::from_secs(10);
    let file_path = root.join("new.tmp");
    fs::write(&file_path, b"new").unwrap();
    File::options()
        .write(true)
        .open(&file_path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(scanned_at - Duration::from_secs(10)))
        .unwrap();
    let policy = CleanupFilePolicy {
        min_age_days: None,
        scanned_at: Some(scanned_at),
    };
    assert_eq!(
        policy.file_decision(&fs::metadata(&file_path).unwrap()),
        CleanupPolicyDecision::ChangedAfterScan
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cargo_config_target_dir_is_discovered() {
    let root = temp_root("cargo-config");
    let cargo_dir = root.join(".cargo");
    let target = root.join("shared-target");
    fs::create_dir_all(target.join("debug")).unwrap();
    fs::create_dir_all(&cargo_dir).unwrap();
    fs::write(
        cargo_dir.join("config.toml"),
        b"[build]\ntarget-dir = \"shared-target\"\n",
    )
    .unwrap();
    let discovered = cargo_target_dir_from_config(&cargo_dir.join("config.toml")).unwrap();
    assert!(same_path(&discovered, &target));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cargo_config_absolute_target_dir_is_preserved() {
    let root = temp_root("cargo-config-absolute");
    let cargo_dir = root.join(".cargo");
    let target = temp_root("cargo-config-external-target");
    fs::create_dir_all(target.join("release")).unwrap();
    fs::create_dir_all(&cargo_dir).unwrap();
    fs::write(
        cargo_dir.join("config.toml"),
        format!(
            "[build]\ntarget-dir = {:?}\n",
            target.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    let discovered = cargo_target_dir_from_config(&cargo_dir.join("config.toml")).unwrap();
    assert!(same_path(&discovered, &target));
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(target).unwrap();
}

#[test]
fn project_level_cargo_target_dir_is_discovered_without_parent_manifest() {
    let root = temp_root("cargo-project-target");
    let cargo_dir = root.join(".cargo");
    let crate_dir = root.join("src-tauri");
    let target = root.join("target");
    fs::create_dir_all(target.join("debug")).unwrap();
    fs::create_dir_all(&crate_dir).unwrap();
    fs::create_dir_all(&cargo_dir).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        b"[package]\nname='sample'\nversion='0.1.0'\n",
    )
    .unwrap();
    fs::write(
        cargo_dir.join("config.toml"),
        b"[build]\ntarget-dir = \"target\"\n",
    )
    .unwrap();

    assert!(is_verified_rust_target(&target, "target"));
    let roots = vec![root.clone()];
    let queue = ProjectScoutQueue::new(&roots);
    let diagnostics = Arc::new(ScanDiagnostics::default());
    let _guard = ActiveScanDiagnostics::install(diagnostics);
    let found = AtomicUsize::new(0);
    let volume_probe_tasks = AtomicUsize::new(0);
    let volume_probe_limit_reported = AtomicBool::new(false);
    let mut discovery = ProjectCacheDiscovery::default();
    let task = queue.pop().unwrap();
    scan_project_cache_task(
        task,
        &roots,
        &queue,
        &found,
        &volume_probe_tasks,
        &volume_probe_limit_reported,
        &mut discovery,
    );
    queue.complete();
    assert!(discovery.rust.iter().any(|path| same_path(path, &target)));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_depth_limit_is_reported() {
    let root = temp_root("depth-warning");
    let child = root.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("file.tmp"), b"data").unwrap();
    let mut result = DirectoryMeasure::default();
    let mut visited = std::collections::HashSet::new();
    dir_size_recursive(&root, &mut result, &mut visited, 0, 0, None);
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.contains("最大扫描深度")));
    fs::remove_dir_all(root).unwrap();
}
