use super::*;

fn gpu(name: &str, backend: Backend, device_type: DeviceType) -> Candidate {
    Candidate {
        name: name.into(),
        backend,
        device_type,
        vendor: 0,
        device: 0,
        pci_bus_id: String::new(),
        driver: String::new(),
        driver_info: String::new(),
        features: 36,
        usable: true,
    }
}

/// #1459: a 9048 px wide window (two display scales' ratio past 8192) fits an adapter that
/// takes 16384 px textures.
#[test]
fn the_surface_may_be_as_large_as_the_adapter_allows() {
    let adapter = wgpu::Limits { max_texture_dimension_2d: 16384, ..wgpu::Limits::default() };
    let asked = wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::default() };
    assert_eq!(device_limits(asked, &adapter).max_texture_dimension_2d, 16384);
}

/// #2105: a GPU that offers less than eframe asks for on any limit (here a Raspberry Pi 5's V3D 7.1:
/// 15 inter-stage variables on Vulkan, 4096 px textures on OpenGL) gets a device with what it
/// offers, instead of failing and leaving the window to the software rasterizer. A GPU that offers
/// more is asked for no more than before, except for the texture side.
#[test]
fn the_device_asks_for_no_more_than_the_adapter_offers() {
    let asked = wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::default() };
    let vulkan = wgpu::Limits { max_inter_stage_shader_variables: 15, max_texture_dimension_2d: 7680, ..wgpu::Limits::default() };
    let limits = device_limits(asked.clone(), &vulkan);
    assert_eq!((limits.max_inter_stage_shader_variables, limits.max_texture_dimension_2d), (15, 7680));
    assert!(limits.check_limits(&vulkan), "{limits:?}");
    let gl = wgpu::Limits { max_texture_dimension_2d: 4096, ..wgpu::Limits::downlevel_webgl2_defaults() };
    let webgl2 = wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::downlevel_webgl2_defaults() };
    let limits = device_limits(webgl2.clone(), &gl);
    assert_eq!(limits.max_texture_dimension_2d, 4096);
    assert!(limits.check_limits(&gl), "{limits:?}");
    let ample = wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::default() };
    assert_eq!(device_limits(asked.clone(), &ample), asked);
    let better = wgpu::Limits { max_inter_stage_shader_variables: 31, max_texture_dimension_2d: 8192, ..wgpu::Limits::default() };
    assert_eq!(device_limits(asked.clone(), &better), asked, "never more than eframe asked for");
}

fn selected(
    candidates: &[Candidate],
    preference: &Preference,
    displays: &[DisplayGpu],
    compositor: Option<(u32, u32)>,
    named: Option<&str>,
    skip: &[String],
) -> Option<usize> {
    selected_with(candidates, preference, (None, named), displays, compositor, skip, &[]).0
}

fn selected_with(
    candidates: &[Candidate],
    preference: &Preference,
    debug: (Option<PowerPreference>, Option<&str>),
    displays: &[DisplayGpu],
    compositor: Option<(u32, u32)>,
    skip: &[String],
    crashed: &[String],
) -> (Option<usize>, Vec<Notice>, bool) {
    let inputs = selection_inputs(candidates, preference, debug, displays, compositor, skip, crashed);
    let eligible: Vec<Candidate> = inputs.indices.iter().map(|&i| candidates[i].clone()).collect();
    match craft_gpu_device::select(&eligible, &inputs.preference) {
        Some(selection) => (inputs.indices.get(selection.index).copied(), selection.notices, inputs.retried),
        None => (None, Vec::new(), inputs.retried),
    }
}

const NVIDIA_3090: (u32, u32) = (0x10de, 0x2204);
const AMD_IGPU: (u32, u32) = (0x1002, 0x164e);
const INTEL_IGPU: (u32, u32) = (0x8086, 0xa780);
const WARP: (u32, u32) = (0x1414, 0x008c);

fn pci_gpu(name: &str, backend: Backend, device_type: DeviceType, pci: (u32, u32)) -> Candidate {
    Candidate { vendor: pci.0, device: pci.1, ..gpu(name, backend, device_type) }
}

fn monitor(pci: (u32, u32)) -> DisplayGpu {
    DisplayGpu { pci, primary: false }
}

fn primary(pci: (u32, u32)) -> DisplayGpu {
    DisplayGpu { pci, primary: true }
}

/// The desktop of pdfcraft#378 as wgpu lists it on Windows: a Ryzen's integrated GPU without a
/// monitor and an RTX 3090 driving both monitors, each under Vulkan and DX12, then WARP and
/// OpenGL (which reports no device id).
fn ryzen_desktop() -> Vec<Candidate> {
    vec![
        pci_gpu("NVIDIA GeForce RTX 3090", Backend::Vulkan, DeviceType::DiscreteGpu, NVIDIA_3090),
        pci_gpu("AMD Radeon(TM) Graphics", Backend::Vulkan, DeviceType::IntegratedGpu, AMD_IGPU),
        pci_gpu("NVIDIA GeForce RTX 3090", Backend::Dx12, DeviceType::DiscreteGpu, NVIDIA_3090),
        pci_gpu("AMD Radeon(TM) Graphics", Backend::Dx12, DeviceType::IntegratedGpu, AMD_IGPU),
        pci_gpu("Microsoft Basic Render Driver", Backend::Dx12, DeviceType::Cpu, WARP),
        pci_gpu("NVIDIA GeForce RTX 3090/PCIe/SSE2", Backend::Gl, DeviceType::Other, (NVIDIA_3090.0, 0)),
    ]
}

/// pdfcraft#378: power saving picked the integrated GPU, whose driver reset when Windows had to
/// present its frames on the monitors of the NVIDIA card. The GPU that drives the displays now
/// comes first as a contextual shared Adapter preference; another GPU remains available after a failure.
#[test]
fn a_desktop_draws_on_the_gpu_that_drives_its_monitors() {
    let c = ryzen_desktop();
    for displays in [vec![primary(NVIDIA_3090)], vec![monitor(NVIDIA_3090)]] {
        assert_eq!(selected(&c, &Preference::Auto, &displays, None, None, &[]), Some(0));
    }
    assert_eq!(selected(&c, &Preference::LowPower, &[], None, None, &[]), Some(1));
    let skip = [key(&c[0]), key(&c[2])];
    assert_eq!(selected(&c, &Preference::Auto, &[primary(NVIDIA_3090)], None, None, &skip), Some(1));
}

/// A hybrid laptop: the panel (primary) on the integrated GPU, an external monitor on the
/// discrete one. Power saving and the display agree, in either enumeration order.
#[test]
fn a_hybrid_laptop_draws_on_the_integrated_gpu_that_drives_its_panel() {
    let c = ryzen_desktop();
    for displays in [vec![primary(AMD_IGPU), monitor(NVIDIA_3090)], vec![monitor(NVIDIA_3090), primary(AMD_IGPU)], vec![primary(AMD_IGPU)]] {
        assert_eq!(selected(&c, &Preference::Auto, &displays, None, None, &[]), Some(1));
    }
    let mut reversed = c.clone();
    reversed.reverse();
    let actual = selected(&reversed, &Preference::Auto, &[primary(AMD_IGPU)], None, None, &[]).unwrap();
    assert_eq!((reversed[actual].vendor, reversed[actual].device), AMD_IGPU);
}

/// A hybrid laptop whose Wayland compositor runs on the discrete GPU (Hyprland with
/// `AQ_DRM_DEVICES` naming it first), as Vulkan lists it there: the integrated GPU drives the
/// built-in panel, yet the compositor shows nothing it draws, and reports no error. The GPU the
/// compositor draws with comes first; without it, the panel's, as before.
#[test]
fn the_gpu_the_compositor_draws_with_comes_before_the_panels() {
    let c = ryzen_desktop();
    let panel = [primary(AMD_IGPU)];
    assert_eq!(selected(&c, &Preference::Auto, &panel, Some(NVIDIA_3090), None, &[]), Some(0));
    assert_eq!(selected(&c, &Preference::Auto, &panel, None, None, &[]), Some(1));
    assert_eq!(selected(&c, &Preference::Auto, &panel, Some(NVIDIA_3090), Some("radeon"), &[]), Some(1));
    assert_eq!(selected(&c, &Preference::Auto, &panel, Some((0x1af4, 0x1050)), None, &[]), Some(1));
    assert_eq!(selected(&c, &Preference::Auto, &panel, Some(NVIDIA_3090), None, &[key(&c[0]), key(&c[2])]), Some(1));
}

/// A Windows desktop with a monitor on each GPU: the window opens on the primary display, so
/// its GPU draws, whichever kind it is.
#[test]
fn the_primary_display_decides_between_gpus_that_both_drive_a_monitor() {
    let c = ryzen_desktop();
    assert_eq!(selected(&c, &Preference::Auto, &[monitor(AMD_IGPU), primary(NVIDIA_3090)], None, None, &[]), Some(0));
    assert_eq!(selected(&c, &Preference::Auto, &[primary(AMD_IGPU), monitor(NVIDIA_3090)], None, None, &[]), Some(1));
    assert_eq!(selected(&c, &Preference::Auto, &[monitor(AMD_IGPU), monitor(INTEL_IGPU)], None, None, &[]), Some(1));
}

/// Without a real matching display the shared policy stays in charge, including an empty list.
#[test]
fn displays_no_adapter_matches_leave_the_power_preference_in_charge() {
    let c = ryzen_desktop();
    for preference in [Preference::Auto, Preference::LowPower] {
        let expected = craft_gpu_device::select(&c, &preference).unwrap().index;
        for displays in [vec![monitor(INTEL_IGPU)], vec![primary((0, 0))], Vec::new()] {
            assert_eq!(selected(&c, &preference, &displays, None, None, &[]), Some(expected));
        }
    }
    assert_eq!(selected(&[], &Preference::Auto, &[primary(NVIDIA_3090)], None, None, &[]), None);
}

/// `WGPU_ADAPTER_NAME` and "can't present" still come before the displays.
#[test]
fn a_named_adapter_comes_before_the_one_driving_the_display() {
    let mut c = ryzen_desktop();
    assert_eq!(selected(&c, &Preference::Auto, &[primary(NVIDIA_3090)], None, Some("RADEON"), &[]), Some(1));
    c[0].usable = false;
    c[2].usable = false;
    assert_eq!(selected(&c, &Preference::Auto, &[primary(NVIDIA_3090)], None, None, &[]), Some(1));
}

/// A user's own choice (Preferences or `WGPU_POWER_PREF`) is kept as it is; only Automatic
/// looks at the displays.
#[test]
fn only_automatic_considers_which_gpu_drives_the_display() {
    assert!(automatic(&Preference::Auto, None));
    for preference in [Preference::LowPower, Preference::Software, Preference::Adapter(AdapterKey::default())] {
        assert!(!automatic(&preference, None));
        assert!(preferred_displays(&preference, None).is_empty());
        assert!(preferred_compositor(&preference, None).is_none());
    }
    for env in [PowerPreference::LowPower, PowerPreference::HighPerformance, PowerPreference::None] {
        assert!(!automatic(&Preference::Auto, Some(env)));
        assert!(preferred_displays(&Preference::Auto, Some(env)).is_empty());
        assert!(preferred_compositor(&Preference::Auto, Some(env)).is_none());
    }
}

#[test]
fn pci_ids_come_from_windows_device_ids() {
    assert_eq!(pci_ids("PCI\\VEN_10DE&DEV_2204&SUBSYS_40421458&REV_A1"), Some(NVIDIA_3090));
    assert_eq!(pci_ids("PCI\\VEN_1002&DEV_164E&SUBSYS_88771043&REV_C1"), Some(AMD_IGPU));
    assert_eq!(pci_ids("pci\\ven_1002&dev_164e"), Some(AMD_IGPU));
    // Remote Desktop and other non-PCI display devices, and malformed ids.
    assert_eq!(pci_ids("ROOT\\BasicDisplay\\0000"), None);
    assert_eq!(pci_ids("PCI\\VEN_10DE&SUBSYS_40421458"), None);
    assert_eq!(pci_ids("PCI\\VEN_10DE&DEV_ZZZZ"), None);
    assert_eq!(pci_ids("VEN_&DEV_"), None);
    assert_eq!(pci_ids(""), None);
}

/// Whatever this machine has: every GPU listed is a PCI device, at most one drives the primary
/// display, and none is listed twice. (A headless CI runner may list none.)
#[cfg(windows)]
#[test]
fn windows_lists_each_gpu_with_a_monitor_once() {
    let gpus = windows_display_gpus();
    assert!(gpus.iter().filter(|g| g.primary).count() <= 1, "{gpus:?}");
    for (i, g) in gpus.iter().enumerate() {
        assert_ne!(g.pci.0, 0, "{gpus:?}");
        assert!(!gpus[..i].iter().any(|h| h.pci == g.pci), "{gpus:?}");
    }
}

#[test]
fn internal_panels_are_edp_lvds_and_dsi_connectors() {
    for name in ["eDP-1", "LVDS-1", "DSI-1"] {
        assert!(is_internal_panel(name), "{name}");
    }
    for name in ["DP-3", "HDMI-A-1", "DVI-D-1", "VGA-1", "Writeback-1", ""] {
        assert!(!is_internal_panel(name), "{name}");
    }
}

/// A sysfs like a hybrid laptop's with an external monitor: the panel and a disconnected port
/// on the integrated GPU (card0), the monitor on the discrete one (card1), and a card without
/// PCI ids. Each GPU is listed once, the one with the panel as primary.
#[test]
fn linux_display_gpus_come_from_the_connected_connectors() -> std::io::Result<()> {
    let temp = tempfile::Builder::new().prefix("vectorcraft-drm-").tempdir()?;
    let dir = temp.path();
    let card = |name: &str, (vendor, device): (u32, u32)| -> std::io::Result<()> {
        std::fs::create_dir_all(dir.join(name).join("device"))?;
        std::fs::write(dir.join(name).join("device/vendor"), format!("{vendor:#06x}\n"))?;
        std::fs::write(dir.join(name).join("device/device"), format!("{device:#06x}\n"))
    };
    let connector = |name: &str, status: &str| -> std::io::Result<()> {
        std::fs::create_dir_all(dir.join(name))?;
        std::fs::write(dir.join(name).join("status"), format!("{status}\n"))
    };
    card("card0", AMD_IGPU)?;
    card("card1", NVIDIA_3090)?;
    std::fs::create_dir_all(dir.join("card2"))?;
    connector("card0-eDP-1", "connected")?;
    connector("card0-HDMI-A-1", "disconnected")?;
    connector("card1-DP-1", "connected")?;
    connector("card1-DP-2", "connected")?;
    connector("card2-Virtual-1", "connected")?;
    std::fs::write(dir.join("renderD128"), "")?;
    let mut gpus = linux_display_gpus(dir);
    gpus.sort_by_key(|g| g.pci);
    assert_eq!(gpus, [primary(AMD_IGPU), monitor(NVIDIA_3090)]);
    assert!(linux_display_gpus(&dir.join("no such dir")).is_empty());
    Ok(())
}

#[test]
fn display_gpus_are_logged_as_pci_ids() {
    assert_eq!(primary(NVIDIA_3090).to_string(), "10de:2204 (primary)");
    assert_eq!(monitor(AMD_IGPU).to_string(), "1002:164e");
    let mut gpus = vec![];
    add_display_gpu(&mut gpus, NVIDIA_3090, false);
    add_display_gpu(&mut gpus, NVIDIA_3090, true);
    add_display_gpu(&mut gpus, AMD_IGPU, false);
    assert_eq!(gpus, [primary(NVIDIA_3090), monitor(AMD_IGPU)]);
}

/// The machine of #502 as Vulkan lists it with Mesa's device-select layer: the GPU KDE's
/// compositor runs on (NVIDIA) first, then the Ryzen's integrated GPU, Mesa's software
/// renderer, and OpenGL.
fn issue_502() -> Vec<Candidate> {
    vec![
        gpu("NVIDIA GeForce RTX 5070", Backend::Vulkan, DeviceType::DiscreteGpu),
        gpu("AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)", Backend::Vulkan, DeviceType::IntegratedGpu),
        gpu("llvmpipe (LLVM 20.1.8, 256 bits)", Backend::Vulkan, DeviceType::Cpu),
        gpu("NVIDIA GeForce RTX 5070/PCIe/SSE2", Backend::Gl, DeviceType::Other),
    ]
}

#[test]
fn automatic_without_a_context_uses_the_shared_policy() {
    let c = issue_502();
    assert_eq!(selected(&c, &Preference::Auto, &[], None, None, &[]), craft_gpu_device::select(&c, &Preference::Auto).map(|s| s.index));
    assert_eq!(effective_preference(&c, &Preference::Auto, None, &[], None, None), Preference::Auto);
}

#[test]
fn shared_preferences_and_missing_choices_keep_the_shared_fallback_notices() {
    let c = ryzen_desktop();
    assert_eq!(selected(&c, &Preference::Auto, &[], None, None, &[]), Some(0));
    assert_eq!(selected(&c, &Preference::LowPower, &[], None, None, &[]), Some(1));
    assert_eq!(selected(&c, &Preference::Software, &[], None, None, &[]), Some(4));
    let missing = AdapterKey { name: "unplugged".into(), backend: "vulkan".into(), vendor: 0x1002, device: 0xffff, ..Default::default() };
    let (actual, notices, _) = selected_with(&c, &Preference::Adapter(missing.clone()), (None, None), &[primary(AMD_IGPU)], None, &[], &[]);
    assert_eq!(actual, Some(0), "missing saved key uses shared Auto, not the panel hint");
    assert!(notices.contains(&Notice::AdapterMissing(missing)));
    let no_software = &c[..4];
    let (_, notices, _) = selected_with(no_software, &Preference::Software, (None, None), &[], None, &[], &[]);
    assert!(notices.contains(&Notice::NoSoftwareAdapter));
}

/// A dual-GPU Mac reports both GPUs under Metal with no PCI ids (#651): their keys still
/// differ, so when the integrated GPU fails, the restart renders on the discrete one.
#[test]
fn gpus_without_pci_ids_are_told_apart_by_name() {
    let c = [gpu("Metal, first", Backend::Metal, DeviceType::IntegratedGpu), gpu("Metal, second", Backend::Metal, DeviceType::IntegratedGpu)];
    assert_ne!(key(&c[0]), key(&c[1]));
    assert_eq!(parse_skip(&encode_skip(&[key(&c[0])])), [key(&c[0])]);
    assert_eq!(selected(&c, &Preference::Auto, &[], None, None, &[key(&c[0])]), Some(1));
}

#[test]
fn a_first_frame_that_never_reaches_the_screen_hangs_its_adapter() {
    let s = Duration::from_secs;
    // The second frame began: the first was presented.
    assert_eq!(first_frame(2, s(60), Some(s(1)), false), FirstFrame::Shown);
    // The first frame built and not yet presented: waited for, then given up on (#964), counting
    // from when its UI pass ended.
    assert_eq!(first_frame(1, s(3), Some(s(1)), false), FirstFrame::Wait);
    assert_eq!(first_frame(1, FIRST_FRAME_TIMEOUT, Some(FIRST_FRAME_TIMEOUT), false), FirstFrame::Hung);
    assert_eq!(first_frame(1, s(45), Some(s(5)), false), FirstFrame::Wait, "a first frame built slowly gets its full time to show");
    assert_eq!(first_frame(1, FIRST_FRAME_TIMEOUT, None, false), FirstFrame::Hung);
    // The first frame still being built: the app's work, not the adapter's (#5160).
    assert_eq!(first_frame(0, s(3), None, false), FirstFrame::Wait);
    assert_eq!(first_frame(0, FIRST_FRAME_TIMEOUT, None, false), FirstFrame::Busy);
    assert_eq!(first_frame(0, s(600), None, false), FirstFrame::Busy, "never a restart without an adapter");
    // A window minimized at the start may not draw again until restored.
    assert_eq!(first_frame(1, s(60), Some(s(60)), true), FirstFrame::Shown);
    // A hung adapter is left out once, like one that failed.
    let skip = vec!["Dx12:10de:28a0".to_string()];
    assert_eq!(restart_without(true, 1, Some("Dx12:10de:28a0"), &skip), None);
    assert_eq!(restart_without(true, 1, Some("Dx12:8086:a7a8"), &skip), Some("Dx12:8086:a7a8"));
}

#[test]
fn a_restart_tries_the_next_adapter() {
    let c = ryzen_desktop();
    let first = selected(&c, &Preference::Auto, &[], None, None, &[]).unwrap();
    let skip = [key(&c[first])];
    let second = selected(&c, &Preference::Auto, &[], None, None, &skip).unwrap();
    assert_ne!(key(&c[first]), key(&c[second]));
    assert_eq!(selected(&c, &Preference::Adapter(AdapterKey::of(&c[first])), &[], None, None, &skip), Some(second));
}

#[test]
fn adapters_that_cannot_present_to_the_window_are_never_tried() {
    let mut c = ryzen_desktop();
    for candidate in &mut c {
        candidate.usable = false;
    }
    assert_eq!(selected(&c, &Preference::Auto, &[], None, None, &[]), None);
    c[4].usable = true;
    assert_eq!(selected(&c, &Preference::Auto, &[primary(NVIDIA_3090)], None, Some("NVIDIA"), &[]), Some(4));
}

#[test]
fn wgpu_adapter_name_picks_an_adapter_first_and_the_rest_stay_as_fallbacks() {
    let c = ryzen_desktop();
    assert_eq!(selected(&c, &Preference::Auto, &[], None, Some("radeon"), &[]), Some(1));
    assert_eq!(selected(&c, &Preference::Auto, &[], None, Some("radeon"), &[key(&c[1])]), Some(3));
    assert_eq!(selected(&c, &Preference::Auto, &[], None, Some("missing"), &[]), Some(0));
}

/// A Windows machine lists every GPU twice (Vulkan and DX12): power saving still takes the
/// integrated GPU first, as wgpu's own choice did (#306), each GPU through DX12 first (#545).
#[test]
fn hybrid_laptop_on_windows_renders_on_the_integrated_gpu_with_power_saving() {
    let c = ryzen_desktop();
    assert_eq!(selected(&c, &Preference::LowPower, &[primary(NVIDIA_3090)], None, None, &[]), Some(1));
    assert_eq!(selected(&c, &Preference::Auto, &[primary(NVIDIA_3090)], None, None, &[]), Some(0));
}

/// The machine of #545: one Intel GPU, whose Vulkan driver made the window flicker black.
/// DX12 comes first whatever the preference; Vulkan stays a fallback, and `WGPU_ADAPTER_NAME`
/// or a restart leaving DX12 out still reach it.
#[test]
fn a_saved_adapter_keeps_its_backend_and_bus_when_the_list_is_reordered() {
    let mut c = ryzen_desktop();
    c[0].pci_bus_id = "0000:01:00.0".into();
    let wanted = AdapterKey::of(&c[0]);
    c.swap(0, 2);
    let index = selected(&c, &Preference::Adapter(wanted.clone()), &[], None, None, &[]).unwrap();
    assert_eq!(AdapterKey::of(&c[index]), wanted, "a saved full key survives enumeration reorder");
    let other_bus = Candidate { pci_bus_id: "0000:02:00.0".into(), ..c[index].clone() };
    c.insert(0, other_bus);
    let index = selected(&c, &Preference::Adapter(wanted.clone()), &[], None, None, &[]).unwrap();
    assert_eq!(AdapterKey::of(&c[index]), wanted);
}

#[test]
fn the_native_descriptor_is_shared_and_saved_choice_does_not_mask_backends() {
    let descriptor = craft_gpu_device::native_descriptor();
    assert!(descriptor.backends.contains(Backends::VULKAN));
    if cfg!(windows) {
        assert!(!descriptor.backends.contains(Backends::GL));
    }
    let c = ryzen_desktop();
    let saved = Preference::Adapter(AdapterKey::of(&c[2]));
    assert_eq!(selected(&c, &saved, &[], None, None, &[]), Some(2), "saved choice picks an adapter, never changes the instance mask");
}

#[test]
fn windows_lists_opengl_once_a_start_leaves_an_adapter_out() {
    let crashed = LeftOut { adapters: vec![key(&issue_2243()[0])], ..LeftOut::default() };
    assert!(windows_opengl(&crashed));
    let listing = LeftOut { opengl_only: true, ..LeftOut::default() };
    assert!(windows_opengl(&listing));
    assert_eq!(listing.backends(Backends::DX12 | Backends::GL), Backends::GL);
}

/// OpenGL stays where it drew before #4150's fix: a GPU without a Direct3D 12 driver (DX12 lists
/// WARP alone) and an adapter `WGPU_ADAPTER_NAME` names; a start with a DX12 GPU lists DX12 alone.
#[test]
fn windows_lists_opengl_without_a_dx12_gpu_or_with_an_adapter_named() {
    let start = LeftOut::default();
    let asked = std::cell::Cell::new(false);
    let hardware = |found: bool| {
        asked.set(true);
        found
    };
    assert!(!opengl_needed(&start, false, false, || hardware(true)), "a DX12 GPU: DX12 alone (#4150)");
    assert!(asked.replace(false));
    assert!(opengl_needed(&start, false, false, || hardware(false)), "WARP alone: the GPU draws through OpenGL, not the CPU");
    assert!(opengl_needed(&start, false, true, || hardware(true)), "a named adapter may be an OpenGL one");
    assert!(opengl_needed(&start, true, false, || hardware(true)), "a restart left an adapter out");
    let listing = LeftOut { opengl_only: true, ..LeftOut::default() };
    asked.set(false);
    assert!(opengl_needed(&listing, false, false, || hardware(true)));
    assert!(!asked.get(), "DX12 isn't listed once OpenGL must be");
    assert!(any_hardware(&[DeviceType::Cpu, DeviceType::IntegratedGpu]));
    assert!(any_hardware(&[DeviceType::VirtualGpu]));
    assert!(!any_hardware(&[DeviceType::Cpu]), "WARP");
    assert!(!any_hardware(&[]), "Windows 7");
}

#[test]
fn power_preference_follows_the_environment_then_the_preference() {
    let c = ryzen_desktop();
    assert_eq!(
        effective_preference(&c, &Preference::Software, Some(PowerPreference::LowPower), &[primary(NVIDIA_3090)], None, None),
        Preference::LowPower
    );
    for env in [PowerPreference::None, PowerPreference::HighPerformance] {
        assert_eq!(effective_preference(&c, &Preference::LowPower, Some(env), &[primary(AMD_IGPU)], None, None), Preference::Auto);
    }
    assert_eq!(effective_preference(&c, &Preference::Software, None, &[], None, None), Preference::Software);
}

/// The preference the engine saves is the one the app reads back before the window opens.
#[test]
fn the_saved_engine_preference_is_found() {
    let mut prefs = vectorcraft_engine::Prefs::default();
    assert_eq!(prefs.gpu_preference, Preference::Auto);
    for preference in [Preference::Auto, Preference::LowPower, Preference::Software, Preference::Adapter(AdapterKey::of(&ryzen_desktop()[2]))] {
        prefs.gpu_preference = preference.clone();
        assert_eq!(prefs.to_json().get("gpuPreference"), Some(&serde_json::json!(preference)));
    }
}

#[test]
fn skipped_adapters_parse_from_a_comma_separated_list() {
    assert_eq!(parse_skip("Vulkan:1002:164e, Gl:10de:2f04,,"), ["Vulkan:1002:164e", "Gl:10de:2f04"]);
    assert!(parse_skip("").is_empty());
    assert!(parse_skip("[{broken").is_empty());
    let mut c = ryzen_desktop();
    c[0].name = "GPU, same label".into();
    c[0].pci_bus_id = "0000:01:00.0".into();
    c[0].driver = "driver, with comma".into();
    let token = key(&c[0]);
    assert_eq!(parse_skip(&encode_skip(&[token.clone()])), [token]);
    let same = Candidate { pci_bus_id: "0000:02:00.0".into(), ..c[0].clone() };
    c.insert(1, same);
    assert!(!excluded(&c, 0, &[legacy_key(&c[0])]), "old ambiguous keys cannot poison two buses");
    assert!(excluded(&c, 0, &[key(&c[0])]));
    assert!(!excluded(&c, 1, &[key(&c[0])]));
}

#[test]
fn panics_in_wgpu_egui_wgpu_and_naga_are_graphics_failures() {
    for file in [
        "/home/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui-wgpu-0.36.2/src/winit.rs",
        r"C:\Users\u\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\wgpu-core-30.0.1\src\device\mod.rs",
        "/rustc/deps/wgpu-hal-30.0.1/src/vulkan/adapter.rs",
        "naga-30.0.0/src/back/spv/writer.rs",
    ] {
        assert!(in_graphics_stack(file), "{file}");
    }
    for file in
        ["apps/vectorcraft/src/main.rs", "/home/wgpu/vectorcraft/crates/ui-egui/src/lib.rs", "egui-0.36.2/src/context.rs", "winit-0.30.12/src/x.rs"]
    {
        assert!(!in_graphics_stack(file), "{file}");
    }
}

#[test]
fn only_a_graphics_failure_while_starting_up_restarts_without_the_adapter() {
    let amd = Some("Vulkan:1002:164e");
    assert_eq!(restart_without(true, 0, amd, &[]), amd);
    assert_eq!(restart_without(true, STARTUP_FRAMES - 1, amd, &[]), amd);
    // After start-up the adapter has shown frames; a lost window is something else.
    assert_eq!(restart_without(true, STARTUP_FRAMES, amd, &[]), None);
    // A bug outside the graphics, or no adapter picked: nothing another adapter would change.
    assert_eq!(restart_without(false, 0, amd, &[]), None);
    assert_eq!(restart_without(true, 0, None, &[]), None);
    // Never the same adapter twice, so restarts end.
    assert_eq!(restart_without(true, 0, amd, &["Vulkan:1002:164e".into()]), None);
}

#[test]
fn a_normal_exit_or_another_failure_does_not_restart() {
    let startup = Startup::default();
    assert_eq!(finish(Ok(Ok(())), &startup), Ok(()));
    // No adapter was picked (none can show the window): the error is returned, nothing restarts.
    let none = eframe::Error::Wgpu(eframe::egui_wgpu::WgpuError::CustomNativeAdapterSelectionError("no graphics adapter can show the window".into()));
    assert_eq!(
        finish(Ok(Err(none)), &startup),
        Err(format!("WGPU error: Adapter selection failed: no graphics adapter can show the window ({HELP})"))
    );
    let _ = startup.adapter.set("Vulkan:1002:164e".into());
    GRAPHICS_PANIC.store(false, Ordering::Relaxed);
    assert_eq!(finish(Err("a bug".into()), &startup), Err("a bug".into()));
}

/// A log folder of its own for each test.
struct TestDir(tempfile::TempDir);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;

    fn deref(&self) -> &Self::Target {
        self.0.path()
    }
}

impl AsRef<std::path::Path> for TestDir {
    fn as_ref(&self) -> &std::path::Path {
        self.0.path()
    }
}

fn start_dir(tag: &str) -> TestDir {
    TestDir(tempfile::Builder::new().prefix(&format!("vectorcraft-gpu-start-{tag}-")).tempdir().unwrap())
}

/// A start that ends frees the start file even while another handle to it is open, as a child
/// process that is starting holds one: the next start reads the file instead of taking it for
/// another copy of the app that is starting.
#[test]
fn an_ended_start_frees_the_start_file_while_a_copy_of_its_handle_is_open() {
    let dir = start_dir("copy");
    let starting = Startup::default();
    starting.track(&dir, &Preference::Auto, false);
    let copy = starting.start.lock().unwrap().as_ref().map(|start| start.file.try_clone().unwrap());
    assert!(copy.is_some(), "the start holds the file");
    starting.ended();
    assert!(StartFile::open(&dir).unwrap().is_some(), "an ended start frees the start file");
    drop(copy);
}

/// What a start in `dir` leaves out; it then ends without crashing.
fn next_start(dir: &std::path::Path, explicit: bool) -> LeftOut {
    let s = Startup::default();
    let left_out = s.track(dir, &Preference::Auto, explicit);
    s.ended();
    left_out
}

/// The machine of #2243: one Intel UHD Graphics 605, listed through DX12 and OpenGL, and
/// Windows' software adapter.
fn issue_2243() -> Vec<Candidate> {
    vec![
        pci_gpu("Intel(R) UHD Graphics 605", Backend::Dx12, DeviceType::IntegratedGpu, (0x8086, 0x3184)),
        pci_gpu("Microsoft Basic Render Driver", Backend::Dx12, DeviceType::Cpu, (0x1414, 0x008c)),
        pci_gpu("Intel(R) UHD Graphics 605", Backend::Gl, DeviceType::Other, (0x8086, 0)),
    ]
}

/// #2243: an exact crashed adapter stays excluded after a successful start on another adapter.
#[test]
fn a_start_that_never_showed_a_frame_leaves_its_adapter_out_of_the_next() {
    let dir = start_dir("crash");
    let c = issue_2243();
    let first = Startup::default();
    assert_eq!(first.track(&dir, &Preference::Auto, false), LeftOut::default());
    first.picked(&AdapterKey::of(&c[0]), &c[0].name, true);
    drop(first);
    let second = Startup::default();
    let left_out = second.track(&dir, &Preference::Auto, false);
    assert_eq!(left_out.adapters, [key(&c[0])]);
    assert_eq!(left_out.crash, Some(Crash::Adapter { key: key(&c[0]), name: c[0].name.clone() }));
    assert!(!left_out.opengl_only);
    assert_eq!(left_out.backends(Backends::DX12 | Backends::GL), Backends::DX12 | Backends::GL);
    let selected = selected(&c, &Preference::Auto, &[], None, None, &left_out.adapters).unwrap();
    assert_ne!(AdapterKey::of(&c[selected]), AdapterKey::of(&c[0]));
    second.picked(&AdapterKey::of(&c[selected]), &c[selected].name, true);
    second.presented();
    let third = Startup::default();
    let left_out = third.track(&dir, &Preference::Auto, false);
    assert_eq!(left_out.adapters, [key(&c[0])], "kept");
    assert_eq!(left_out.crash, None, "the note is shown once");
    third.ended();
}

/// A start whose first frame reached the screen, or that ended without crashing (a normal exit,
/// an error, a restart on another adapter), leaves nothing out of the next.
#[test]
fn a_start_that_showed_its_first_frame_or_ended_is_cleared() {
    let dir = start_dir("shown");
    let c = issue_2243();
    for end in [Startup::presented as fn(&Startup), Startup::ended] {
        let first = Startup::default();
        first.track(&dir, &Preference::Auto, false);
        first.picked(&AdapterKey::of(&c[0]), &c[0].name, true);
        end(&first);
        drop(first);
        assert_eq!(next_start(&dir, false), LeftOut::default());
    }
    // So does a start that ended before it picked an adapter.
    let first = Startup::default();
    first.track(&dir, &Preference::Auto, false);
    assert!(finish(Ok(Ok(())), &first).is_ok());
    drop(first);
    assert_eq!(next_start(&dir, false), LeftOut::default());
}

/// The process ended while the adapters were being listed (wgpu's DX12 backend creates a
/// device on each GPU to list it): the next starts list OpenGL's adapters alone, except on
/// macOS, where wgpu has none.
#[test]
fn a_crash_while_listing_lists_opengl_alone() {
    let dir = start_dir("listing");
    let first = Startup::default();
    first.track(&dir, &Preference::Auto, false);
    drop(first);
    let second = Startup::default();
    let left_out = second.track(&dir, &Preference::Auto, false);
    assert!(left_out.adapters.is_empty());
    if cfg!(target_os = "macos") {
        assert_eq!(left_out, LeftOut::default());
    } else {
        assert_eq!(left_out.crash, Some(Crash::Listing));
        assert_eq!(left_out.backends(Backends::DX12 | Backends::GL), Backends::GL);
        assert_eq!(left_out.backends(Backends::PRIMARY | Backends::GL), Backends::GL);
        assert_eq!(left_out.backends(Backends::METAL), Backends::METAL, "no OpenGL to fall back to");
    }
    // Listing OpenGL alone found nothing that can show the window: the next start lists every
    // backend again.
    second.none_left();
    second.ended();
    assert_eq!(next_start(&dir, false), LeftOut::default());
}

/// An explicit backend or adapter debug choice ignores historical crashes; when that
/// start shows its first frame on the adapter that crashed, the adapter counts again.
#[test]
fn wgpu_backend_or_adapter_name_ignores_the_crashes() {
    let dir = start_dir("explicit");
    let c = issue_2243();
    let first = Startup::default();
    first.track(&dir, &Preference::Auto, false);
    first.picked(&AdapterKey::of(&c[0]), &c[0].name, true);
    drop(first);
    assert_eq!(next_start(&dir, true), LeftOut::default());
    assert_eq!(next_start(&dir, false).adapters, [key(&c[0])], "still left out without them");

    let explicit = Startup::default();
    explicit.track(&dir, &Preference::Auto, true);
    explicit.picked(&AdapterKey::of(&c[0]), &c[0].name, true);
    explicit.presented();
    assert_eq!(next_start(&dir, false), LeftOut::default(), "a new driver shows the window");
}

/// A damaged record, a file that can't be opened and one another starting copy of the app holds
/// are ignored: the start goes on with nothing left out.
#[test]
fn a_damaged_or_unreadable_start_file_is_ignored() {
    let dir = start_dir("damaged");
    std::fs::create_dir_all(&dir).unwrap();
    let damaged: [&[u8]; 9] = [
        b"",
        b"{",
        b"[]",
        b"null",
        b"{\"version\":1}",
        b"{\"starting\":{}}",
        b"\xef\xbb\xbf\0",
        b"\xff\xfe{",
        &[b' '; START_FILE_LIMIT as usize + 1],
    ];
    for text in damaged {
        std::fs::write(dir.join(START_FILE), text).unwrap();
        assert_eq!(next_start(&dir, false), LeftOut::default(), "{:?}", String::from_utf8_lossy(&text[..text.len().min(20)]));
    }

    // Another copy of the app is starting: this start neither reads nor writes the file.
    let c = issue_2243();
    let starting = Startup::default();
    starting.track(&dir, &Preference::Auto, false);
    starting.picked(&AdapterKey::of(&c[0]), &c[0].name, true);
    assert_eq!(next_start(&dir, false), LeftOut::default());
    drop(starting);
    assert_eq!(next_start(&dir, false).adapters, [key(&c[0])]);

    // A folder in the file's place.
    std::fs::remove_file(dir.join(START_FILE)).unwrap();
    std::fs::create_dir_all(dir.join(START_FILE)).unwrap();
    assert_eq!(next_start(&dir, false), LeftOut::default());
}

#[test]
fn crashes_are_forgotten_in_another_version_or_under_another_preference() {
    let actual = AdapterKey::of(&ryzen_desktop()[0]);
    let preference = Preference::Adapter(actual.clone());
    let (mut record, crash) = StartRecord::begin(None, "0.9.0", &preference);
    assert_eq!(crash, None);
    record.picked(&actual, "GPU", true);
    let (record, crash) = StartRecord::begin(Some(record), "0.9.0", &preference);
    assert_eq!(crash, Some(Crash::Adapter { key: adapter_fingerprint(&actual), name: "GPU".into() }));
    assert_eq!(record.left_out(None, false).adapters, [adapter_fingerprint(&actual)]);
    assert_eq!(StartRecord::parse(&record.to_json()), Some(record.clone()));
    assert!(StartRecord::begin(Some(record.clone()), "0.9.1", &preference).0.crashed.is_empty());
    for other in [
        Preference::Auto,
        Preference::LowPower,
        Preference::Software,
        Preference::Adapter(AdapterKey { backend: "dx12".into(), ..actual.clone() }),
        Preference::Adapter(AdapterKey { pci_bus_id: "other bus".into(), ..actual.clone() }),
        Preference::Adapter(AdapterKey { driver: "other driver".into(), ..actual.clone() }),
    ] {
        let (changed, crash) = StartRecord::begin(Some(record.clone()), "0.9.0", &other);
        assert_eq!(crash, None);
        assert!(changed.crashed.is_empty(), "{other:?}");
    }
    let mut record = record;
    for n in 0..CRASHED_LIMIT + 4 {
        let adapter = AdapterKey { device: n as u32, ..actual.clone() };
        for _ in 0..2 {
            record.picked(&adapter, "GPU", true);
            record = StartRecord::begin(Some(record), "0.9.0", &preference).0;
        }
    }
    assert_eq!(record.crashed.len(), CRASHED_LIMIT);
    assert_eq!(record.crashed.last().map(|c| &c.key), Some(&adapter_fingerprint(&AdapterKey { device: 19, ..actual.clone() })));
}

/// `u32::from_str_radix` accepts a leading `+`; a device id with one is not a PCI id.
#[test]
fn pci_ids_reject_a_sign_in_the_hex_digits() {
    assert_eq!(pci_ids("PCI\\VEN_+10DE&DEV_2204"), None);
    assert_eq!(pci_ids("PCI\\VEN_10DE&DEV_+2204"), None);
}

/// #9183: a Mac lists one adapter. A start ended from outside before its first frame left it
/// recorded as crashed, and the next start left it out and had nothing to draw on. It is tried
/// again when nothing else is left; an adapter a restart left out after it failed stays out, and
/// with another adapter to try, the one that crashed is still left out.
#[test]
fn an_adapter_that_crashed_an_earlier_start_is_tried_again_when_no_other_is_left() {
    let mac = [gpu("Apple M6", Backend::Metal, DeviceType::IntegratedGpu)];
    let crashed = vec![key(&mac[0])];
    let (index, _, retried) = selected_with(&mac, &Preference::Auto, (None, None), &[], None, &[], &crashed);
    assert_eq!((index, retried), (Some(0), true));
    let (index, _, retried) = selected_with(&mac, &Preference::Auto, (None, None), &[], None, &crashed, &crashed);
    assert_eq!((index, retried), (None, false), "a failure in this restart chain is never retried");
    let two = [mac[0].clone(), gpu("llvmpipe", Backend::Gl, DeviceType::Cpu)];
    let (index, _, retried) = selected_with(&two, &Preference::Auto, (None, None), &[], None, &[], &crashed);
    assert_eq!((index, retried), (Some(1), false));
}

#[test]
fn crash_and_restart_keys_do_not_cross_backend_bus_or_driver_identity() {
    let mut candidates = ryzen_desktop();
    candidates[0].pci_bus_id = "0000:01:00.0".into();
    candidates[0].driver = "driver one".into();
    let original = candidates[0].clone();
    let other_bus = Candidate { pci_bus_id: "0000:02:00.0".into(), ..original.clone() };
    let other_driver = Candidate { driver: "driver two".into(), ..original.clone() };
    let other_backend = Candidate { backend: Backend::Dx12, ..original.clone() };
    let all = [original.clone(), other_bus, other_driver, other_backend];
    let crashed = [key(&original)];
    assert!(excluded(&all, 0, &crashed));
    for i in 1..all.len() {
        assert!(!excluded(&all, i, &crashed), "full key {i} is different");
    }
    let full = AdapterKey::of(&original);
    assert_ne!(adapter_fingerprint(&full), full.to_string(), "Display is only a human label");
    let roundtrip = parse_skip(&encode_skip(&crashed));
    assert_eq!(roundtrip, crashed);
}

#[test]
fn legacy_start_records_reset_without_poisoning_structured_preferences() {
    let legacy = serde_json::json!({"version":"v", "gpuPreference":"automatic", "gpuBackend":"vulkan",
        "crashed":[{"key":"Vulkan:10de:2204","name":"GPU"}], "listingCrashed":true,
        "starting":{"adapter":{"key":"Vulkan:10de:2204","name":"GPU"},"native":true}});
    assert_eq!(StartRecord::parse(&legacy.to_string()), None, "lossy legacy crash identity is reset, not applied to a new structured preference");
    let (record, crash) = StartRecord::begin(None, "v", &Preference::Auto);
    assert_eq!(crash, None);
    assert!(record.crashed.is_empty());
    assert_eq!(record.preference, preference_fingerprint(&Preference::Auto));
    assert_eq!(StartRecord::parse(&record.to_json()), Some(record));
}

#[test]
fn excluding_a_corrected_driver_never_resurrects_its_stale_sibling() {
    let old = Candidate { driver: "AMD proprietary driver".into(), features: 60, ..ryzen_desktop()[1].clone() };
    let newer = Candidate { pci_bus_id: "0000:03:00.0".into(), features: 68, ..old.clone() };
    let dx12 = Candidate { backend: Backend::Dx12, ..newer.clone() };
    let other_driver = Candidate { driver: "Mesa alternate".into(), ..newer.clone() };
    let all = [old.clone(), newer.clone(), dx12, other_driver];
    let policy = eligible_inventory(&all);
    assert_eq!(policy.iter().map(|c| c.usable).collect::<Vec<_>>(), [false, true, true, true]);
    assert_ne!(key(&old), key(&newer), "a missing bus and a reported bus are different crash identities");
    let clean = selection_inputs(&all, &Preference::Auto, (None, None), &[], None, &[], &[]);
    assert!(clean.indices.contains(&1));
    assert!(!clean.indices.contains(&0));
    for (skip, crashed) in [(vec![key(&newer)], vec![]), (vec![], vec![key(&newer)])] {
        let inputs = selection_inputs(&all, &Preference::Auto, (None, None), &[], None, &skip, &crashed);
        assert_eq!(inputs.indices, [2, 3], "neither exclusion route resurrects stale Vulkan");
        assert!(!inputs.retried, "the other backend and independent driver remain available");
    }
    let only_duplicates = [old, newer.clone()];
    let (_, _, retried) = selected_with(&only_duplicates, &Preference::Auto, (None, None), &[], None, &[], &[key(&newer)]);
    assert!(retried, "historical crash retry still permits the corrected adapter when no alternative remains");
    assert_eq!(selected(&only_duplicates, &Preference::Auto, &[], None, None, &[key(&newer)]), None, "restart exclusions are never retried");
}

#[test]
fn preferences_inventory_marks_shared_unsafe_and_unpresentable_entries_unavailable() {
    let unsafe_vulkan =
        Candidate { driver: "Intel Windows".into(), ..pci_gpu("Intel UHD 630", Backend::Vulkan, DeviceType::IntegratedGpu, (0x8086, 0x3e91)) };
    let dx12 = Candidate { backend: Backend::Dx12, ..unsafe_vulkan.clone() };
    let valid = ryzen_desktop()[0].clone();
    let hidden = Candidate { usable: false, ..valid.clone() };
    for all in [vec![unsafe_vulkan.clone(), valid.clone()], vec![unsafe_vulkan, dx12, valid, hidden]] {
        let policy = eligible_inventory(&all);
        assert!(!policy[0].usable, "surface support alone does not make unsafe Intel Vulkan eligible");
        assert!(policy[1].usable);
        for (actual, shown) in all.iter().zip(&policy) {
            assert_eq!(AdapterKey::of(actual), AdapterKey::of(shown), "full identity is retained for missing/disabled choice display");
            if !actual.usable {
                assert!(!shown.usable, "shared eligibility never overrides surface rejection");
            }
        }
    }
}

#[test]
fn oversized_complete_identities_fit_and_roundtrip_the_recovery_record() {
    let huge = "\u{0001}é".repeat(30_000);
    let actual =
        AdapterKey { name: huge.clone(), backend: "vulkan".into(), vendor: 0x10de, device: 0x2204, pci_bus_id: huge.clone(), driver: huge.clone() };
    let preference = Preference::Adapter(actual.clone());
    let (mut record, _) = StartRecord::begin(None, &huge, &preference);
    for device in 0..CRASHED_LIMIT as u32 {
        record.picked(&AdapterKey { device, ..actual.clone() }, &huge, true);
        record = StartRecord::begin(Some(record), &huge, &preference).0;
    }
    record.picked(&actual, &huge, true);
    let text = record.to_json();
    assert!(text.len() < START_FILE_LIMIT as usize, "the whole record, not just its labels, fits the reader: {}", text.len());
    assert_eq!(StartRecord::parse(&text), Some(record.clone()));
    assert_eq!(record.crashed.len(), CRASHED_LIMIT);
    assert!(record.crashed.iter().all(|r| r.name.len() <= 128));
    let (restarted, crash) = StartRecord::begin(StartRecord::parse(&text), &huge, &preference);
    assert!(matches!(crash, Some(Crash::Adapter { ref key, .. }) if *key == adapter_fingerprint(&actual)));
    assert!(restarted.left_out(None, false).adapters.contains(&adapter_fingerprint(&actual)));
    let other = Preference::Adapter(AdapterKey { driver: format!("{huge}x"), ..actual.clone() });
    let (changed, crash) = StartRecord::begin(Some(record), &huge, &other);
    assert_eq!(crash, None, "a change beyond the bounded human label resets the complete preference scope");
    assert!(changed.crashed.is_empty());

    let small = AdapterKey::of(&ryzen_desktop()[0]);
    let schema2 = serde_json::json!({
        "schema": 2, "version": "v", "gpuPreference": Preference::Adapter(small.clone()),
        "crashed": [{"key": small, "name": "GPU"}], "listingCrashed": false,
        "starting": {"adapter": {"key": small, "name": "GPU"}, "native": true}
    });
    let migrated = StartRecord::parse(&schema2.to_string()).unwrap();
    assert_eq!(migrated.preference, preference_fingerprint(&Preference::Adapter(small.clone())));
    assert_eq!(migrated.crashed[0].key, adapter_fingerprint(&small));
    assert_eq!(StartRecord::parse(&migrated.to_json()), Some(migrated));
    let old_skip = serde_json::json!([small]).to_string();
    assert_eq!(parse_skip(&old_skip), [adapter_fingerprint(&small)], "old structured restart arrays retain their identity");
}
