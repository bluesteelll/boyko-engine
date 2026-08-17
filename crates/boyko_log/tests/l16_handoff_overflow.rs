//! L16: the ECS handoff's refusals are counted AND reported -- never silent.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, info};

#[test]
fn a_refusing_handoff_reports_its_pass_count_and_says_the_byte_sinks_are_intact() {
    let path = std::env::temp_dir().join("boyko_l16_handoff.log");
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::file::set_path(path.to_str().expect("a UTF-8 temp path")));
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: true,
        file: true,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));
    boyko_log::sink::slot::reset();

    // Fill the handoff far past its capacity WITHOUT consuming it: nothing plays the
    // `log_drain_system` role here, so every frame after the ring is full is refused.
    let (lost_before, _) = boyko_log::sink::ecs::lost();
    for i in 0..4096u32 {
        info!(Log, "overflow probe {}", i);
        if i % 64 == 63 {
            // Drained every 64 so the LANE does not overflow instead -- that would measure the
            // wrong ring, which is exactly the mistake L12's control leg made.
            let DrainResult::Ran(_) = drain() else { panic!("the drain role is free") };
        }
    }
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free") };
    let (lost_after, _) = boyko_log::sink::ecs::lost();
    assert!(lost_after > lost_before, "the handoff never refused; the probe did not fill it");

    let text = std::fs::read_to_string(&path).expect("the sink's file is readable");
    let code = format!("boyko-W{:04}", boyko_log::codes::W0117.number());
    assert!(text.contains(&code), "a refusing handoff emitted no {code}");
    assert!(
        text.contains("the byte sinks are not"),
        "W0117 must say the FILE is intact -- a reader seeing a gap in a widget cannot otherwise \
         tell 'the engine stopped reporting' from 'the widget could not keep up'"
    );

    // AND THE RECORDS THEMSELVES ARE IN THE FILE. The refusal is about the in-frame view only, so
    // a probe refused by the handoff must still be readable here -- otherwise the warning's own
    // claim is false.
    assert!(
        text.contains("overflow probe 4095"),
        "a record the handoff refused is missing from the byte sink too"
    );

    let _ = std::fs::remove_file(&path);
}
