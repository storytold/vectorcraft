use super::*;

fn gpu(name: &str, backend: Backend, device_type: DeviceType) -> Candidate {
    Candidate { key: format!("{backend:?}:{name}"), name: name.into(), backend, device_type, pci: (0, 0), presents: true }
}

/// #1459: a 9048 px wide window (two display scales' ratio past 8192) fits an adapter that
/// takes 16384 px textures; an adapter with less than eframe asks for changes nothing.
#[test]
fn the_surface_may_be_as_large_as_the_adapter_allows() {
    assert_eq!(surface_limit(8192, 16384), 16384);
    assert_eq!(surface_limit(8192, 4096), 8192);
}

fn names(c: &[Candidate], order: &[usize]) -> Vec<String> {
    order.iter().map(|&i| c[i].name.clone()).collect()
}

const NVIDIA_3090: (u32, u32) = (0x10de, 0x2204);
const AMD_IGPU: (u32, u32) = (0x1002, 0x164e);
const INTEL_IGPU: (u32, u32) = (0x8086, 0xa780);
const WARP: (u32, u32) = (0x1414, 0x008c);

fn pci_gpu(name: &str, backend: Backend, device_type: DeviceType, pci: (u32, u32)) -> Candidate {
    Candidate { key: key(backend, pci.0, pci.1, name), name: name.into(), backend, device_type, pci, presents: true }
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
/// comes first, through DX12 (#545); the integrated GPU stays the fallback a restart reaches.
#[test]
fn a_desktop_draws_on_the_gpu_that_drives_its_monitors() {
    let c = ryzen_desktop();
    for displays in [vec![primary(NVIDIA_3090)], vec![monitor(NVIDIA_3090)]] {
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, None, &[]), [2, 0, 3, 1, 5, 4], "{displays:?}");
        assert_eq!(adapter_order(&c, PowerPreference::None, &displays, None, None, &[]), [2, 0, 3, 1, 5, 4], "{displays:?}");
    }
    // Without the display information the order is power saving's, as before.
    assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[]), [3, 1, 2, 0, 5, 4]);
    // After the NVIDIA adapters failed, the integrated GPU is next.
    let skip = vec![c[2].key.clone(), c[0].key.clone()];
    assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[primary(NVIDIA_3090)], None, None, &skip), [3, 1, 5, 4]);
}

/// A hybrid laptop: the panel (primary) on the integrated GPU, an external monitor on the
/// discrete one. Power saving and the display agree, in either enumeration order.
#[test]
fn a_hybrid_laptop_draws_on_the_integrated_gpu_that_drives_its_panel() {
    let c = ryzen_desktop();
    for displays in [vec![primary(AMD_IGPU), monitor(NVIDIA_3090)], vec![monitor(NVIDIA_3090), primary(AMD_IGPU)], vec![primary(AMD_IGPU)]] {
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, None, &[]), [3, 1, 2, 0, 5, 4], "{displays:?}");
    }
    // Linux names no primary unless there is a built-in panel: a monitor on each GPU keeps the
    // system's order among them (Mesa puts the compositor's GPU first), and a GPU without one
    // comes after both.
    let linux = vec![
        pci_gpu("AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)", Backend::Vulkan, DeviceType::IntegratedGpu, AMD_IGPU),
        pci_gpu("NVIDIA GeForce RTX 3090", Backend::Vulkan, DeviceType::DiscreteGpu, NVIDIA_3090),
        pci_gpu("Intel(R) Arc(TM) A380", Backend::Vulkan, DeviceType::DiscreteGpu, INTEL_IGPU),
        pci_gpu("llvmpipe (LLVM 20.1.8, 256 bits)", Backend::Vulkan, DeviceType::Cpu, (0x10005, 0)),
    ];
    let both = [monitor(AMD_IGPU), monitor(NVIDIA_3090)];
    assert_eq!(adapter_order(&linux, PowerPreference::None, &both, None, None, &[]), [0, 1, 2, 3]);
    assert_eq!(adapter_order(&linux, PowerPreference::None, &[monitor(INTEL_IGPU)], None, None, &[]), [2, 0, 1, 3]);
    assert_eq!(adapter_order(&linux, PowerPreference::None, &[monitor(NVIDIA_3090), primary(AMD_IGPU)], None, None, &[]), [0, 1, 2, 3]);
}

/// A hybrid laptop whose Wayland compositor runs on the discrete GPU (Hyprland with
/// `AQ_DRM_DEVICES` naming it first), as Vulkan lists it there: the integrated GPU drives the
/// built-in panel, yet the compositor shows nothing it draws, and reports no error. The GPU the
/// compositor draws with comes first; without it, the panel's, as before.
#[test]
fn the_gpu_the_compositor_draws_with_comes_before_the_panels() {
    const RTX_3050: (u32, u32) = (0x10de, 0x25ac);
    const INTEL_RPL: (u32, u32) = (0x8086, 0xa7a8);
    let mut c = vec![
        pci_gpu("NVIDIA GeForce RTX 3050 6GB Laptop GPU", Backend::Vulkan, DeviceType::DiscreteGpu, RTX_3050),
        pci_gpu("Intel(R) Graphics (RPL-P)", Backend::Vulkan, DeviceType::IntegratedGpu, INTEL_RPL),
        pci_gpu("NVIDIA GeForce RTX 3050 6GB Laptop GPU/PCIe/SSE2", Backend::Gl, DeviceType::Other, (RTX_3050.0, 0)),
    ];
    c[2].presents = false;
    let panel = [primary(INTEL_RPL)];
    assert_eq!(adapter_order(&c, PowerPreference::None, &panel, Some(RTX_3050), None, &[]), [0, 1]);
    assert_eq!(adapter_order(&c, PowerPreference::None, &panel, None, None, &[]), [1, 0]);
    // The usual hybrid laptop, with the compositor on the integrated GPU: the panel's GPU.
    assert_eq!(adapter_order(&c, PowerPreference::None, &[primary(INTEL_RPL), monitor(RTX_3050)], Some(INTEL_RPL), None, &[]), [1, 0]);
    // `WGPU_ADAPTER_NAME` still comes first, and a restart after a failure still moves on.
    assert_eq!(adapter_order(&c, PowerPreference::None, &panel, Some(RTX_3050), Some("intel"), &[]), [1, 0]);
    assert_eq!(adapter_order(&c, PowerPreference::None, &panel, Some(RTX_3050), None, &[c[0].key.clone()]), [1]);
    // A compositor GPU no adapter matches (a virtual one) leaves the displays in charge.
    assert_eq!(adapter_order(&c, PowerPreference::None, &panel, Some((0x1af4, 0x1050)), None, &[]), [1, 0]);
}

/// A Windows desktop with a monitor on each GPU: the window opens on the primary display, so
/// its GPU draws, whichever kind it is.
#[test]
fn the_primary_display_decides_between_gpus_that_both_drive_a_monitor() {
    let c = ryzen_desktop();
    assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[monitor(AMD_IGPU), primary(NVIDIA_3090)], None, None, &[]), [2, 0, 3, 1, 5, 4]);
    assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[primary(AMD_IGPU), monitor(NVIDIA_3090)], None, None, &[]), [3, 1, 2, 0, 5, 4]);
    // The displays only matter among adapters that can show the window; a GPU driving no
    // display still loses to one that does.
    assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[monitor(AMD_IGPU), monitor(INTEL_IGPU)], None, None, &[]), [3, 1, 2, 0, 5, 4]);
}

/// Display GPUs none of the adapters match (another driver's ids, OpenGL's missing device id,
/// Metal's zeros) change nothing: power saving's order, the old behaviour.
#[test]
fn displays_no_adapter_matches_leave_the_power_preference_in_charge() {
    let c = ryzen_desktop();
    let low = adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[]);
    for displays in [vec![monitor(INTEL_IGPU)], vec![primary((0x8086, 0x1234)), monitor((0x8086, 0x5678))], vec![primary((0, 0))]] {
        assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, None, &[]), low, "{displays:?}");
    }
    assert_eq!(
        names(&c, &adapter_order(&c, PowerPreference::HighPerformance, &[monitor(INTEL_IGPU)], None, None, &[]))[0],
        "NVIDIA GeForce RTX 3090"
    );
    // The #502 machine's adapters report no ids in these tests: the primary display never matches.
    let c = issue_502();
    assert_eq!(
        adapter_order(&c, PowerPreference::None, &[primary(NVIDIA_3090)], None, None, &[]),
        adapter_order(&c, PowerPreference::None, &[], None, None, &[])
    );
    assert!(adapter_order(&[], PowerPreference::LowPower, &[primary(NVIDIA_3090)], None, None, &[]).is_empty());
}

/// `WGPU_ADAPTER_NAME` and "can't present" still come before the displays.
#[test]
fn a_named_adapter_comes_before_the_one_driving_the_display() {
    let mut c = ryzen_desktop();
    let displays = [primary(NVIDIA_3090)];
    assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, Some("radeon"), &[]), [3, 1, 2, 0, 5, 4]);
    c[2].presents = false;
    c[0].presents = false;
    assert_eq!(adapter_order(&c, PowerPreference::LowPower, &displays, None, None, &[]), [3, 1, 5, 4]);
}

/// A user's own choice (Preferences or `WGPU_POWER_PREF`) is kept as it is; only Automatic
/// looks at the displays.
#[test]
fn only_automatic_considers_which_gpu_drives_the_display() {
    assert!(automatic(None, None));
    assert!(automatic(Some("automatic"), None));
    assert!(automatic(Some("powerSaving"), None));
    assert!(!automatic(Some("lowPower"), None));
    assert!(!automatic(Some("highPerformance"), None));
    assert!(!automatic(None, Some(PowerPreference::None)));
    assert!(!automatic(Some("automatic"), Some(PowerPreference::LowPower)));
    assert!(preferred_displays(Some("lowPower"), None).is_empty());
    assert!(preferred_displays(None, Some(PowerPreference::HighPerformance)).is_empty());
    // Automatic reports whatever this machine has (nothing on macOS, in CI, or under Remote Desktop).
    assert_eq!(preferred_displays(None, None), display_gpus());
    assert_eq!(preferred_compositor(Some("lowPower"), None), None);
    assert_eq!(preferred_compositor(None, Some(PowerPreference::HighPerformance)), None);
    assert_eq!(preferred_compositor(None, None), crate::compositor_gpu::compositor_gpu());
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
    let dir = std::env::temp_dir().join(format!("vectorcraft-drm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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
    let mut gpus = linux_display_gpus(&dir);
    gpus.sort_by_key(|g| g.pci);
    assert_eq!(gpus, [primary(AMD_IGPU), monitor(NVIDIA_3090)]);
    assert!(linux_display_gpus(&dir.join("no such dir")).is_empty());
    std::fs::remove_dir_all(&dir)
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
fn automatic_keeps_the_systems_order_on_linux_and_saves_power_elsewhere() {
    let c = issue_502();
    let order = adapter_order(&c, power_preference(None, None), &[], None, None, &[]);
    let first = &c[order[0]].name;
    if cfg!(any(windows, target_os = "macos")) {
        assert_eq!(first, "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)");
    } else {
        // #502: the integrated GPU couldn't show frames to a compositor on the NVIDIA GPU.
        assert_eq!(first, "NVIDIA GeForce RTX 5070");
    }
}

#[test]
fn each_preference_orders_the_adapters_then_falls_back_to_opengl_and_software() {
    let c = issue_502();
    let order = |power| names(&c, &adapter_order(&c, power, &[], None, None, &[]));
    assert_eq!(
        order(PowerPreference::None),
        [
            "NVIDIA GeForce RTX 5070",
            "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)",
            "NVIDIA GeForce RTX 5070/PCIe/SSE2",
            "llvmpipe (LLVM 20.1.8, 256 bits)"
        ]
    );
    assert_eq!(
        order(PowerPreference::LowPower),
        [
            "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)",
            "NVIDIA GeForce RTX 5070",
            "NVIDIA GeForce RTX 5070/PCIe/SSE2",
            "llvmpipe (LLVM 20.1.8, 256 bits)"
        ]
    );
    assert_eq!(
        order(PowerPreference::HighPerformance),
        [
            "NVIDIA GeForce RTX 5070",
            "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)",
            "NVIDIA GeForce RTX 5070/PCIe/SSE2",
            "llvmpipe (LLVM 20.1.8, 256 bits)"
        ]
    );
}

/// A restart leaves out the adapter the window failed on, so Power Saving on the machine of
/// #502 ends up on the NVIDIA GPU, and once every adapter failed there is nothing to try.
/// A dual-GPU Mac reports both GPUs under Metal with no PCI ids (#651): their keys still
/// differ, so when the integrated GPU fails, the restart renders on the discrete one.
#[test]
fn gpus_without_pci_ids_are_told_apart_by_name() {
    let mac = |name: &str, device_type| Candidate {
        key: key(Backend::Metal, 0, 0, name),
        name: name.into(),
        backend: Backend::Metal,
        device_type,
        pci: (0, 0),
        presents: true,
    };
    let c = vec![mac("NVIDIA GeForce GT 750M", DeviceType::DiscreteGpu), mac("Intel Iris Pro Graphics", DeviceType::IntegratedGpu)];
    assert_eq!((c[0].key.as_str(), c[1].key.as_str()), ("Metal:NVIDIA GeForce GT 750M", "Metal:Intel Iris Pro Graphics"));
    let first = adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[]);
    assert_eq!(names(&c, &first)[0], "Intel Iris Pro Graphics");
    let skip = parse_skip(&c[first[0]].key);
    assert_eq!(names(&c, &adapter_order(&c, PowerPreference::LowPower, &[], None, None, &skip)), ["NVIDIA GeForce GT 750M"]);
    // A comma in a name can't split the list.
    assert_eq!(parse_skip(&key(Backend::Gl, 0, 0, "Mesa, llvmpipe")), ["Gl:Mesa  llvmpipe"]);
}

#[test]
fn a_first_frame_that_never_reaches_the_screen_hangs_its_adapter() {
    let s = Duration::from_secs;
    // The second frame began: the first was presented.
    assert_eq!(first_frame(2, s(60), false), FirstFrame::Shown);
    // Still drawing (or presenting) the first frame: waited for, then given up on (#964).
    for frames in [0, 1] {
        assert_eq!(first_frame(frames, s(3), false), FirstFrame::Wait);
        assert_eq!(first_frame(frames, FIRST_FRAME_TIMEOUT, false), FirstFrame::Hung);
    }
    // A window minimized at the start may not draw again until restored.
    assert_eq!(first_frame(1, s(60), true), FirstFrame::Shown);
    // A hung adapter is left out once, like one that failed.
    let skip = vec!["Dx12:10de:28a0".to_string()];
    assert_eq!(restart_without(true, 1, Some("Dx12:10de:28a0"), &skip), None);
    assert_eq!(restart_without(true, 1, Some("Dx12:8086:a7a8"), &skip), Some("Dx12:8086:a7a8"));
}

#[test]
fn a_restart_tries_the_next_adapter() {
    let c = issue_502();
    let mut skip = vec![];
    let mut tried = vec![];
    while let Some(&i) = adapter_order(&c, PowerPreference::LowPower, &[], None, None, &skip).first() {
        tried.push(c[i].name.clone());
        skip.push(c[i].key.clone());
    }
    assert_eq!(
        tried,
        [
            "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)",
            "NVIDIA GeForce RTX 5070",
            "NVIDIA GeForce RTX 5070/PCIe/SSE2",
            "llvmpipe (LLVM 20.1.8, 256 bits)"
        ]
    );
}

#[test]
fn adapters_that_cannot_present_to_the_window_are_never_tried() {
    let mut c = issue_502();
    c[0].presents = false;
    c[3].presents = false;
    for power in [PowerPreference::None, PowerPreference::LowPower, PowerPreference::HighPerformance] {
        let order = adapter_order(&c, power, &[], None, None, &[]);
        assert!(!order.contains(&0) && !order.contains(&3), "{power:?}: {order:?}");
        assert_eq!(order.len(), 2);
    }
    for p in &mut c {
        p.presents = false;
    }
    assert!(adapter_order(&c, PowerPreference::None, &[], None, None, &[]).is_empty());
    assert!(adapter_order(&[], PowerPreference::HighPerformance, &[], None, None, &[]).is_empty());
}

#[test]
fn wgpu_adapter_name_picks_an_adapter_first_and_the_rest_stay_as_fallbacks() {
    let c = issue_502();
    let order = adapter_order(&c, PowerPreference::HighPerformance, &[], None, Some("radv"), &[]);
    assert_eq!(c[order[0]].name, "AMD Ryzen 9 7900X (RADV RAPHAEL_MENDOCINO)");
    assert_eq!(order.len(), 4);
    // Any case; every match comes before the rest, natively first.
    let order = adapter_order(&c, PowerPreference::LowPower, &[], None, Some("GeForce"), &[]);
    assert_eq!(names(&c, &order[..2]), ["NVIDIA GeForce RTX 5070", "NVIDIA GeForce RTX 5070/PCIe/SSE2"]);
    // An empty name or one nothing matches changes nothing.
    for named in ["", "no such gpu"] {
        assert_eq!(
            adapter_order(&c, PowerPreference::LowPower, &[], None, Some(named), &[]),
            adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[])
        );
    }
    // A named adapter that failed is left out like any other.
    let skip = vec![c[1].key.clone()];
    assert_eq!(adapter_order(&c, PowerPreference::None, &[], None, Some("radv"), &skip)[0], 0);
}

/// A Windows machine lists every GPU twice (Vulkan and DX12): power saving still takes the
/// integrated GPU first, as wgpu's own choice did (#306), each GPU through DX12 first (#545).
#[test]
fn hybrid_laptop_on_windows_renders_on_the_integrated_gpu_with_power_saving() {
    let c = vec![
        gpu("Intel(R) Arc(TM) A370M", Backend::Vulkan, DeviceType::DiscreteGpu),
        gpu("Intel(R) Iris(R) Xe Graphics", Backend::Vulkan, DeviceType::IntegratedGpu),
        gpu("Intel(R) Arc(TM) A370M", Backend::Dx12, DeviceType::DiscreteGpu),
        gpu("Intel(R) Iris(R) Xe Graphics", Backend::Dx12, DeviceType::IntegratedGpu),
        gpu("Microsoft Basic Render Driver", Backend::Dx12, DeviceType::Cpu),
    ];
    let order = adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[]);
    assert_eq!(order, [3, 1, 2, 0, 4]);
    assert_eq!(adapter_order(&c, PowerPreference::HighPerformance, &[], None, None, &[]), [2, 0, 3, 1, 4]);
}

/// The machine of #545: one Intel GPU, whose Vulkan driver made the window flicker black.
/// DX12 comes first whatever the preference; Vulkan stays a fallback, and `WGPU_ADAPTER_NAME`
/// or a restart leaving DX12 out still reach it.
#[test]
fn a_windows_gpu_renders_through_dx12_before_vulkan() {
    let c = vec![
        gpu("Intel(R) Graphics", Backend::Vulkan, DeviceType::IntegratedGpu),
        gpu("Intel(R) Graphics", Backend::Dx12, DeviceType::IntegratedGpu),
        gpu("Microsoft Basic Render Driver", Backend::Dx12, DeviceType::Cpu),
        gpu("Intel(R) Graphics", Backend::Gl, DeviceType::Other),
    ];
    for power in [PowerPreference::LowPower, PowerPreference::HighPerformance, PowerPreference::None] {
        assert_eq!(adapter_order(&c, power, &[], None, None, &[]), [1, 0, 3, 2], "{power:?}");
    }
    assert_eq!(adapter_order(&c, PowerPreference::LowPower, &[], None, None, &[c[1].key.clone()]), [0, 3, 2], "DX12 failed: Vulkan next");
}

#[test]
fn windows_never_loads_vulkan_drivers_unless_asked() {
    let all = Backends::all();
    let start = backends(all, false);
    if cfg!(windows) {
        assert_eq!(start, Backends::DX12 | Backends::GL, "#806");
    } else {
        assert_eq!(start, all);
    }
    assert_eq!(backends(Backends::VULKAN, true), Backends::VULKAN, "WGPU_BACKEND=vulkan still picks it");
}

#[test]
fn power_preference_follows_the_environment_then_the_preference() {
    assert_eq!(power_preference(None, None), AUTOMATIC);
    assert_eq!(power_preference(Some("automatic"), None), AUTOMATIC);
    assert_eq!(power_preference(Some("lowPower"), None), PowerPreference::LowPower);
    assert_eq!(power_preference(Some("highPerformance"), None), PowerPreference::HighPerformance);
    // 0.5.0's default, and values this version doesn't know (a newer or damaged file).
    for old in ["powerSaving", "turbo", ""] {
        assert_eq!(power_preference(Some(old), None), AUTOMATIC, "{old}");
    }
    // WGPU_POWER_PREF wins over the preference, either way.
    assert_eq!(power_preference(Some("lowPower"), Some(PowerPreference::HighPerformance)), PowerPreference::HighPerformance);
    assert_eq!(power_preference(Some("highPerformance"), Some(PowerPreference::LowPower)), PowerPreference::LowPower);
    assert_eq!(power_preference(Some("lowPower"), Some(PowerPreference::None)), PowerPreference::None);
    assert_eq!(AUTOMATIC, if cfg!(any(windows, target_os = "macos")) { PowerPreference::LowPower } else { PowerPreference::None });
}

/// The preference the engine saves is the one the app reads back before the window opens.
#[test]
fn the_saved_engine_preference_is_found() {
    let mut prefs = vectorcraft_engine::Prefs::default();
    let saved = prefs.to_json();
    assert_eq!(saved.get("gpuPreference").and_then(serde_json::Value::as_str), Some("automatic"));
    for (value, power) in [("lowPower", PowerPreference::LowPower), ("highPerformance", PowerPreference::HighPerformance)] {
        prefs.gpu_preference = value.into();
        let saved = prefs.to_json();
        assert_eq!(power_preference(saved.get("gpuPreference").and_then(serde_json::Value::as_str), None), power);
    }
    // Every choice Preferences offer means something here.
    for (value, _) in vectorcraft_engine::cmd::prefscmds::GPU_PREFERENCES {
        assert!(["automatic", "lowPower", "highPerformance"].contains(value), "{value}");
    }
}

#[test]
fn skipped_adapters_parse_from_a_comma_separated_list() {
    assert_eq!(parse_skip("Vulkan:1002:164e, Gl:10de:2f04,,"), ["Vulkan:1002:164e", "Gl:10de:2f04"]);
    assert!(parse_skip("").is_empty());
    assert_eq!(key(Backend::Vulkan, 0x1002, 0x164e, "AMD Radeon"), "Vulkan:1002:164e");
    assert_eq!(key(Backend::Gl, 0x10de, 0x2f04, "NVIDIA"), "Gl:10de:2f04");
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
fn start_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vectorcraft-gpu-start-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// What a start in `dir` leaves out; it then ends without crashing.
fn next_start(dir: &std::path::Path, explicit: bool) -> LeftOut {
    let s = Startup::default();
    let left_out = s.track(dir, None, explicit);
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

/// #2243: the driver ends the process after the adapter was picked, before the first frame.
/// The next start leaves that adapter out and draws through OpenGL, and so do the starts after
/// it, without a status-bar note.
#[test]
fn a_start_that_never_showed_a_frame_leaves_its_adapter_out_of_the_next() {
    let dir = start_dir("crash");
    let c = issue_2243();
    let first = Startup::default();
    assert_eq!(first.track(&dir, None, false), LeftOut::default());
    first.picked(&c[0].key, &c[0].name, true);
    // The process ends here: its file is closed and unlocked, and the start is still recorded.
    drop(first);

    let second = Startup::default();
    let left_out = second.track(&dir, Some("automatic"), false);
    assert_eq!(left_out.adapters, ["Dx12:8086:3184"]);
    assert_eq!(left_out.crash, Some(Crash::Adapter { key: "Dx12:8086:3184".into(), name: "Intel(R) UHD Graphics 605".into() }));
    assert!(!left_out.opengl_only);
    assert_eq!(left_out.backends(Backends::DX12 | Backends::GL), Backends::DX12 | Backends::GL);
    let order = adapter_order(&c, AUTOMATIC, &[], None, None, &left_out.adapters);
    assert_eq!(order, [2, 1], "OpenGL on the same GPU before the software adapter");
    second.picked(&c[2].key, &c[2].name, true);
    second.presented();

    let third = Startup::default();
    let left_out = third.track(&dir, None, false);
    assert_eq!(left_out.adapters, ["Dx12:8086:3184"], "kept");
    assert_eq!(left_out.crash, None, "the note is shown once");
    third.ended();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A start whose first frame reached the screen, or that ended without crashing (a normal exit,
/// an error, a restart on another adapter), leaves nothing out of the next.
#[test]
fn a_start_that_showed_its_first_frame_or_ended_is_cleared() {
    let dir = start_dir("shown");
    let c = issue_2243();
    for end in [Startup::presented as fn(&Startup), Startup::ended] {
        let first = Startup::default();
        first.track(&dir, None, false);
        first.picked(&c[0].key, &c[0].name, true);
        end(&first);
        drop(first);
        assert_eq!(next_start(&dir, false), LeftOut::default());
    }
    // So does a start that ended before it picked an adapter.
    let first = Startup::default();
    first.track(&dir, None, false);
    assert!(finish(Ok(Ok(())), &first).is_ok());
    drop(first);
    assert_eq!(next_start(&dir, false), LeftOut::default());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The process ended while the adapters were being listed (wgpu's DX12 backend creates a
/// device on each GPU to list it): the next starts list OpenGL's adapters alone, except on
/// macOS, where wgpu has none.
#[test]
fn a_crash_while_listing_lists_opengl_alone() {
    let dir = start_dir("listing");
    let first = Startup::default();
    first.track(&dir, None, false);
    drop(first);
    let second = Startup::default();
    let left_out = second.track(&dir, None, false);
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
    let _ = std::fs::remove_dir_all(&dir);
}

/// `WGPU_BACKEND` or `WGPU_ADAPTER_NAME` choose for a start whatever crashed before; when that
/// start shows its first frame on the adapter that crashed, the adapter counts again.
#[test]
fn wgpu_backend_or_adapter_name_ignores_the_crashes() {
    let dir = start_dir("explicit");
    let c = issue_2243();
    let first = Startup::default();
    first.track(&dir, None, false);
    first.picked(&c[0].key, &c[0].name, true);
    drop(first);
    assert_eq!(next_start(&dir, true), LeftOut::default());
    assert_eq!(next_start(&dir, false).adapters, ["Dx12:8086:3184"], "still left out without them");

    let explicit = Startup::default();
    explicit.track(&dir, None, true);
    explicit.picked(&c[0].key, &c[0].name, true);
    explicit.presented();
    assert_eq!(next_start(&dir, false), LeftOut::default(), "a new driver shows the window");
    let _ = std::fs::remove_dir_all(&dir);
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
    starting.track(&dir, None, false);
    starting.picked(&c[0].key, &c[0].name, true);
    assert_eq!(next_start(&dir, false), LeftOut::default());
    drop(starting);
    assert_eq!(next_start(&dir, false).adapters, ["Dx12:8086:3184"]);

    // A folder in the file's place.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(START_FILE)).unwrap();
    assert_eq!(next_start(&dir, false), LeftOut::default());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn crashes_are_forgotten_in_another_version_or_under_another_preference() {
    let starting = |key: &str| Starting { adapter: Some(Recorded { key: key.into(), name: "GPU".into() }), native: true };
    let (mut record, crash) = StartRecord::begin(None, "0.9.0", "automatic");
    assert_eq!(crash, None);
    record.starting = Some(starting("Dx12:8086:3184"));
    let (record, crash) = StartRecord::begin(Some(record), "0.9.0", "automatic");
    assert_eq!(crash, Some(Crash::Adapter { key: "Dx12:8086:3184".into(), name: "GPU".into() }));
    assert_eq!(record.left_out(None, false).adapters, ["Dx12:8086:3184"]);
    // Written and read back.
    assert_eq!(StartRecord::parse(&record.to_json()), Some(record.clone()));
    assert!(StartRecord::begin(Some(record.clone()), "0.9.1", "automatic").0.crashed.is_empty());
    assert!(StartRecord::begin(Some(record.clone()), "0.9.0", "highPerformance").0.crashed.is_empty());
    assert_eq!(preference_name(None), "automatic");
    assert_eq!(preference_name(Some("powerSaving")), "automatic");
    assert_eq!(preference_name(Some("lowPower")), "lowPower");
    // An adapter is listed once, and the list is bounded.
    let mut record = record;
    for n in 0..CRASHED_LIMIT + 4 {
        for _ in 0..2 {
            record.starting = Some(starting(&format!("Dx12:10de:{n:04x}")));
            record = StartRecord::begin(Some(record), "0.9.0", "automatic").0;
        }
    }
    assert_eq!(record.crashed.len(), CRASHED_LIMIT);
    assert_eq!(record.crashed.last().map(|c| c.key.as_str()), Some("Dx12:10de:0013"));
}
