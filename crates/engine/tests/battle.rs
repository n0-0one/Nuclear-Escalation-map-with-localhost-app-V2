//! End-to-end checks: a simulated battle built from the real mission data is
//! fed through the same parsing path the game uses, and the engine's picture
//! is compared with what really happened in the simulation.

use ntl_engine::convoy::ConvoyState;
use ntl_engine::ident::Confidence;
use ntl_engine::sim::Sim;
use ntl_engine::world::{Proof, Status};
use ntl_engine::{Engine, Pack};
use std::path::PathBuf;
use std::sync::Arc;

fn pack() -> Arc<Pack> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    Arc::new(Pack::load(&dir).expect("data pack loads"))
}

/// Runs the simulation and the engine side by side.
fn run(sim: &mut Sim, engine: &mut Engine, seconds: f64, mut each: impl FnMut(&Sim, &Engine)) {
    let step = 0.4;
    let mut last_hud = 0;
    let mut elapsed = 0.0;
    while elapsed < seconds {
        sim.step(step);
        elapsed += step;
        let frame = sim.frame(last_hud);
        last_hud = sim.last_hud_id();
        engine.ingest(&frame, sim.t + 1000.0);
        each(sim, engine);
    }
}

#[test]
fn pack_is_sane() {
    let pack = pack();
    assert!(pack.levels.len() >= 3);
    assert!(pack.core.eras.len() >= 5);
    assert!(pack.core.vehicles.len() > 1000);
    assert!(pack.core.hud.names.len() > 500);
    for level in &pack.levels {
        assert_eq!(level.variants.len(), 2, "{} has both layouts", level.id);
        assert!(!level.road_file.roads.is_empty(), "{} has roads", level.id);
    }
}

#[test]
fn recognises_every_map_side_and_bracket() {
    let pack = pack();
    let mut checked = 0;
    for level in 0..pack.levels.len() {
        for variant in 0..2 {
            for team in 1..=2u8 {
                for era in 0..pack.core.eras.len() {
                    let mut sim = Sim::new(pack.clone(), level, variant, era, team, (level * 100 + variant * 10 + era) as u64 + team as u64);
                    let mut engine = Engine::new(pack.clone());
                    run(&mut sim, &mut engine, 12.0, |_, _| {});
                    let id = engine.identity();
                    let label = format!("{} {} team {team} era {era}", pack.levels[level].id, variant);
                    assert_eq!(id.level, Some(level), "map: {label}: {:?}", id.why);
                    assert_eq!(id.variant, Some(variant), "layout: {label}: {:?}", id.why);
                    assert_eq!(id.team, Some(team), "team: {label}: {:?}", id.why);
                    assert_eq!(id.era, Some(era), "bracket: {label}: {:?} options {:?}", id.why, id.era_options);
                    assert!(matches!(id.side_conf, Confidence::Confirmed | Confidence::Likely), "{label}");
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, pack.levels.len() * 2 * 2 * pack.core.eras.len());
}

#[test]
fn tracks_destroyed_units_without_inventing_any() {
    let pack = pack();
    let mut sim = Sim::new(pack.clone(), 0, 0, 3, 1, 42);
    let mut engine = Engine::new(pack.clone());
    run(&mut sim, &mut engine, 900.0, |_, _| {});
    let world = engine.world().expect("battle identified");
    let truth = |site: u16, slot: u16| sim.units.iter().find(|u| u.site == site && u.slot == slot).unwrap();

    let (mut confirmed, mut confirmed_wrong, mut likely, mut likely_wrong) = (0, 0, 0, 0);
    for unit in world.units.iter().filter(|u| u.status == Status::Destroyed) {
        let dead = !truth(unit.site, unit.slot).alive;
        match unit.proof {
            Proof::Confirmed => {
                confirmed += 1;
                confirmed_wrong += usize::from(!dead);
            }
            _ => {
                likely += 1;
                likely_wrong += usize::from(!dead);
            }
        }
    }
    let enemy_dead = sim.units.iter().filter(|u| !u.alive && u.team == 2).count();
    println!("enemy dead {enemy_dead}; confirmed {confirmed} (wrong {confirmed_wrong}); likely {likely} (wrong {likely_wrong}); unlocated {}", world.unlocated_kills());
    assert!(confirmed >= 10, "expected the kill feed to confirm kills, got {confirmed}");
    assert!(confirmed_wrong * 20 <= confirmed, "confirmed kills must be right: {confirmed_wrong}/{confirmed}");
    assert!(likely_wrong * 4 <= likely.max(4), "likely kills should mostly be right: {likely_wrong}/{likely}");
    // Units the engine calls alive must be alive.
    let alive_wrong = world
        .units
        .iter()
        .filter(|u| u.status == Status::Alive && u.visible(sim.t + 1000.0) && !truth(u.site, u.slot).alive)
        .count();
    assert_eq!(alive_wrong, 0);
}

#[test]
fn predicts_convoys_after_they_drop_off_the_map() {
    let pack = pack();
    let mut errors: Vec<f64> = Vec::new();
    let mut observed_any = false;
    let mut alerts = 0usize;
    for seed in [7u64, 19, 23] {
        let mut sim = Sim::new(pack.clone(), 0, 0, 4, 1, seed);
        let mut engine = Engine::new(pack.clone());
        let mut tick = 0u32;
        run(&mut sim, &mut engine, 1200.0, |sim, engine| {
            tick += 1;
            if tick % 25 != 0 {
                return;
            }
            let convoys = engine.convoys();
            for convoy in &convoys.list {
                if convoy.team != 2 {
                    continue;
                }
                observed_any |= convoy.state == ConvoyState::Observed;
                if convoy.state != ConvoyState::Predicted || convoy.arrived_at.is_some() {
                    continue;
                }
                let unseen = sim.t + 1000.0 - convoy.last_seen;
                if !(30.0..=400.0).contains(&unseen) {
                    continue;
                }
                // distance to the nearest real enemy column
                let nearest = sim.convoys.iter().filter(|c| c.team == 2).map(|c| c.head().dist(convoy.pos)).fold(f64::INFINITY, f64::min);
                if nearest.is_finite() {
                    errors.push(nearest);
                }
            }
            alerts += engine.snapshot().alerts.len();
        });
    }
    assert!(observed_any, "the engine should have followed at least one enemy column");
    assert!(errors.len() >= 20, "expected predictions to check, got {}", errors.len());
    errors.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = errors[errors.len() / 2];
    let p80 = errors[errors.len() * 8 / 10];
    println!("convoy prediction error: median {median:.0} m, 80% under {p80:.0} m over {} samples; alert frames {alerts}", errors.len());
    assert!(median < 700.0, "median prediction error too large: {median:.0} m");
    assert!(alerts > 0, "a column passing a depot or strongpoint should raise an alert");
}

#[test]
fn snapshot_serialises_and_reports_nearby_air_defence() {
    let pack = pack();
    let mut sim = Sim::new(pack.clone(), 1, 1, 4, 2, 5);
    let mut engine = Engine::new(pack.clone());
    run(&mut sim, &mut engine, 60.0, |_, _| {});
    let snapshot = engine.snapshot();
    assert_eq!(snapshot.link, "battle");
    assert!(snapshot.player.is_some());
    assert!(!snapshot.cues.is_empty(), "nearest SAM sites are listed");
    assert!(snapshot.cues.windows(2).all(|w| w[0].dist_m <= w[1].dist_m));
    assert!(snapshot.battle.clock_s.is_some() || sim.last_hud_id() == 0);
    let text = serde_json::to_string(&snapshot).unwrap();
    assert!(text.len() > 500 && text.len() < 600_000, "snapshot size {}", text.len());
}

#[test]
fn a_new_battle_clears_the_old_one() {
    let pack = pack();
    let mut engine = Engine::new(pack.clone());
    let mut first = Sim::new(pack.clone(), 0, 0, 2, 1, 1);
    run(&mut first, &mut engine, 30.0, |_, _| {});
    assert_eq!(engine.identity().level, Some(0));
    let mut second = Sim::new(pack.clone(), 2, 1, 4, 2, 2);
    run(&mut second, &mut engine, 30.0, |_, _| {});
    let id = engine.identity();
    assert_eq!((id.level, id.variant, id.team, id.era), (Some(2), Some(1), Some(2), Some(4)), "{:?}", id.why);
}
