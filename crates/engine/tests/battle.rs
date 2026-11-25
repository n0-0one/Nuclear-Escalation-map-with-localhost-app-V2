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
                    let mut sim = Sim::new(
                        pack.clone(),
                        level,
                        variant,
                        era,
                        team,
                        (level * 100 + variant * 10 + era) as u64 + team as u64,
                    );
                    let mut engine = Engine::new(pack.clone());
                    run(&mut sim, &mut engine, 12.0, |_, _| {});
                    let id = engine.identity();
                    let label = format!(
                        "{} {} team {team} era {era}",
                        pack.levels[level].id, variant
                    );
                    assert_eq!(id.level, Some(level), "map: {label}: {:?}", id.why);
                    assert_eq!(id.variant, Some(variant), "layout: {label}: {:?}", id.why);
                    assert_eq!(id.team, Some(team), "team: {label}: {:?}", id.why);
                    assert_eq!(
                        id.era,
                        Some(era),
                        "bracket: {label}: {:?} options {:?}",
                        id.why,
                        id.era_options
                    );
                    assert!(
                        matches!(id.side_conf, Confidence::Confirmed | Confidence::Likely),
                        "{label}"
                    );
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
    let truth = |site: u16, slot: u16| {
        sim.units
            .iter()
            .find(|u| u.site == site && u.slot == slot)
            .unwrap()
    };

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
    assert!(
        confirmed >= 10,
        "expected the kill feed to confirm kills, got {confirmed}"
    );
    assert!(
        confirmed_wrong * 20 <= confirmed,
        "confirmed kills must be right: {confirmed_wrong}/{confirmed}"
    );
    assert!(
        likely_wrong * 4 <= likely.max(4),
        "likely kills should mostly be right: {likely_wrong}/{likely}"
    );
    // Units the engine calls alive must be alive.
    let alive_wrong = world
        .units
        .iter()
        .filter(|u| {
            u.status == Status::Alive && u.visible(sim.t + 1000.0) && !truth(u.site, u.slot).alive
        })
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
                let nearest = sim
                    .convoys
                    .iter()
                    .filter(|c| c.team == 2)
                    .map(|c| c.head().dist(convoy.pos))
                    .fold(f64::INFINITY, f64::min);
                if nearest.is_finite() {
                    errors.push(nearest);
                }
            }
            alerts += engine.snapshot().alerts.len();
        });
    }
    assert!(
        observed_any,
        "the engine should have followed at least one enemy column"
    );
    assert!(
        errors.len() >= 20,
        "expected predictions to check, got {}",
        errors.len()
    );
    errors.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = errors[errors.len() / 2];
    let p80 = errors[errors.len() * 8 / 10];
    println!("convoy prediction error: median {median:.0} m, 80% under {p80:.0} m over {} samples; alert frames {alerts}", errors.len());
    assert!(
        median < 700.0,
        "median prediction error too large: {median:.0} m"
    );
    assert!(
        alerts > 0,
        "a column passing a depot or strongpoint should raise an alert"
    );
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
    assert!(
        text.len() > 500 && text.len() < 600_000,
        "snapshot size {}",
        text.len()
    );
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
    assert_eq!(
        (id.level, id.variant, id.team, id.era),
        (Some(2), Some(1), Some(2), Some(4)),
        "{:?}",
        id.why
    );
}

/// The first real match showed that detection must not depend on any single
/// clue. Here airfields carry no team colour and every ground unit is 70 m
/// away from where the mission data puts it.
#[test]
fn finds_the_side_when_the_usual_clues_are_missing() {
    let pack = pack();
    for level in 0..pack.levels.len() {
        for variant in 0..2 {
            for team in 1..=2u8 {
                let mut sim = Sim::new(
                    pack.clone(),
                    level,
                    variant,
                    2,
                    team,
                    900 + (level * 10 + variant) as u64 + team as u64,
                );
                sim.neutral_airfields = true;
                sim.position_error_m = 70.0;
                let mut engine = Engine::new(pack.clone());
                run(&mut sim, &mut engine, 15.0, |_, _| {});
                let id = engine.identity();
                let label = format!("{} layout {variant} team {team}", pack.levels[level].id);
                assert_eq!(id.level, Some(level), "map: {label}: {:?}", id.why);
                assert_eq!(id.variant, Some(variant), "layout: {label}: {:?}", id.why);
                assert_eq!(id.team, Some(team), "team: {label}: {:?}", id.why);
                assert!(
                    engine.world().is_some(),
                    "tracking starts even on a best guess: {label}"
                );
            }
        }
    }
}

#[test]
fn reports_the_players_own_marks() {
    let pack = pack();
    let mut sim = Sim::new(pack.clone(), 2, 1, 2, 2, 77);
    let mut engine = Engine::new(pack.clone());
    run(&mut sim, &mut engine, 10.0, |_, _| {});
    let snapshot = engine.snapshot();
    assert_eq!(
        snapshot.contacts.iter().filter(|c| c.kind == "poi").count(),
        1
    );
    assert_eq!(
        snapshot.contacts.iter().filter(|c| c.kind == "aim").count(),
        4
    );
    assert!(snapshot
        .diag
        .seen
        .iter()
        .any(|(name, _)| name.starts_with("point_of_interest")));
    assert!(snapshot.diag.detection.fixed_friendly > 50);
}

/// Ships sail their routes; the app follows them while seen and estimates
/// where they are while not.
#[test]
fn ships_sail_and_are_followed() {
    let pack = pack();
    let mut sim = Sim::new(pack.clone(), 0, 0, 4, 1, 31);
    // Fly over the enemy fleet and away again, so its ships come and go from the map.
    let fleet: Vec<_> = sim
        .units
        .iter()
        .filter(|u| u.route.is_some() && u.team == 2)
        .map(|u| u.pos)
        .collect();
    let home = sim.player().0;
    sim.set_player_route(vec![
        home,
        fleet[0],
        fleet[fleet.len() / 2],
        fleet[fleet.len() - 1],
    ]);
    let mut engine = Engine::new(pack.clone());
    let (mut seen_err, mut guess_err) = (Vec::new(), Vec::new());
    let mut tick = 0;
    run(&mut sim, &mut engine, 1500.0, |sim, engine| {
        tick += 1;
        if tick % 20 != 0 {
            return;
        }
        let snapshot = engine.snapshot();
        for ship in &snapshot.ships {
            let Some(real) = sim
                .units
                .iter()
                .find(|u| u.site == ship.site && u.route.is_some())
            else {
                continue;
            };
            if !real.alive || ship.destroyed {
                continue;
            }
            let error = real.pos.dist(ship.p);
            match ship.state {
                "observed" => seen_err.push(error),
                "predicted" => guess_err.push(error),
                _ => {}
            }
        }
    });
    let median = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
    };
    let (seen, guess) = (median(&mut seen_err), median(&mut guess_err));
    println!("ships: on the map median error {seen:.0} m ({} samples), estimated {guess:.0} m ({} samples)", seen_err.len(), guess_err.len());
    assert!(
        seen_err.len() > 50 && seen < 50.0,
        "seen ships are where the game shows them"
    );
    assert!(
        guess_err.len() > 20 && guess < 2500.0,
        "estimated ships stay near their real position"
    );
}

#[test]
fn sunk_ships_are_marked_destroyed() {
    let pack = pack();
    let mut sim = Sim::new(pack.clone(), 0, 0, 4, 1, 32);
    sim.kill_every = (1e9, 1e9); // only the kills made here
    let fleet: Vec<_> = sim
        .units
        .iter()
        .filter(|u| u.route.is_some() && u.team == 2)
        .map(|u| u.pos)
        .collect();
    sim.set_player_route(vec![fleet[0], fleet[fleet.len() - 1]]);
    let mut engine = Engine::new(pack.clone());
    run(&mut sim, &mut engine, 30.0, |_, _| {});
    // one enemy ship that is on the map, and one friendly ship
    let mut sunk = Vec::new();
    for team in [2u8, 1] {
        let snapshot = engine.snapshot();
        let index = (0..sim.units.len())
            .find(|&i| {
                let u = &sim.units[i];
                u.route.is_some()
                    && u.team == team
                    && snapshot
                        .ships
                        .iter()
                        .any(|s| s.site == u.site && s.state == "observed")
            })
            .expect("a ship on the map");
        sim.destroy_unit(index);
        sunk.push((sim.units[index].site, sim.units[index].slot));
        run(&mut sim, &mut engine, 15.0, |_, _| {});
    }
    run(&mut sim, &mut engine, 20.0, |_, _| {});
    let world = engine.world().unwrap();
    for (site, slot) in sunk {
        let unit = world
            .units
            .iter()
            .find(|u| u.site == site && u.slot == slot)
            .unwrap();
        assert_eq!(
            unit.status,
            Status::Destroyed,
            "ship at site {site} is shown as destroyed"
        );
    }
    assert!(engine.snapshot().ships.iter().any(|s| s.destroyed));
}

#[test]
fn captured_sites_change_hands() {
    let pack = pack();
    let mut sim = Sim::new(pack.clone(), 0, 0, 3, 1, 33);
    sim.kill_every = (1e9, 1e9);
    let mut engine = Engine::new(pack.clone());
    run(&mut sim, &mut engine, 30.0, |_, _| {});
    let variant = &pack.levels[0].variants[0];
    // an enemy strongpoint taken by the player's team: its new garrison is friendly and always on the map
    let site = variant
        .sites
        .iter()
        .position(|s| s.team == 2 && s.cat == "strongpoint")
        .unwrap();
    for index in 0..sim.units.len() {
        if sim.units[index].site as usize == site {
            sim.destroy_unit(index);
        }
    }
    run(&mut sim, &mut engine, 20.0, |_, _| {});
    sim.capture_site(site, 1);
    run(&mut sim, &mut engine, 20.0, |_, _| {});
    let snapshot = engine.snapshot();
    assert!(
        snapshot
            .owners
            .iter()
            .any(|&(s, team, _)| s as usize == site && team == 1),
        "owners: {:?}",
        snapshot.owners
    );
    assert!(snapshot.log.iter().any(|l| l.text.contains("captured")));
    // and it can be set back by hand
    engine.capture(site as u16, None);
    assert!(engine.world().unwrap().owner[site] == 2);
}

/// Real icons can sit tens of metres away from the mission positions.
#[test]
fn tracks_destroyed_units_with_offset_positions() {
    let pack = pack();
    let mut sim = Sim::new(pack.clone(), 1, 0, 2, 2, 44);
    sim.position_error_m = 50.0;
    let mut engine = Engine::new(pack.clone());
    run(&mut sim, &mut engine, 900.0, |_, _| {});
    let world = engine.world().expect("battle identified");
    let truth = |site: u16, slot: u16| {
        sim.units
            .iter()
            .find(|u| u.site == site && u.slot == slot)
            .unwrap()
    };
    let enemy_dead = sim
        .units
        .iter()
        .filter(|u| !u.alive && u.team == 1 && u.route.is_none())
        .count();
    let found = world
        .units
        .iter()
        .filter(|u| u.status == Status::Destroyed && u.team == 1 && !truth(u.site, u.slot).alive)
        .count();
    let wrong = world
        .units
        .iter()
        .filter(|u| u.status == Status::Destroyed && truth(u.site, u.slot).alive)
        .count();
    println!("offset 50 m: enemy dead {enemy_dead}, found {found}, wrong {wrong}");
    // Without an offset the same battle places about half of the kills (the
    // rest hit units nobody on the team could see); the offset must not cost more.
    assert!(
        found * 10 >= enemy_dead * 5,
        "kills are placed as well as without an offset"
    );
    assert!(
        world.offset.dist(ntl_engine::geo::V2(43.5, 25.0)) < 10.0,
        "the map-wide offset is learned: {:?}",
        world.offset
    );
    assert!(wrong * 10 <= found.max(10), "few wrong calls");
}

#[test]
fn unplaced_kills_list_the_right_suspects() {
    let pack = pack();
    let mut sim = Sim::new(pack.clone(), 1, 0, 2, 2, 44);
    let mut engine = Engine::new(pack.clone());
    run(&mut sim, &mut engine, 900.0, |_, _| {});
    let snapshot = engine.snapshot();
    let world = engine.world().expect("battle identified");
    let suspects = world.suspects(1e12);
    let listed: std::collections::HashSet<u32> =
        suspects.iter().flat_map(|s| s.2.iter().copied()).collect();
    let (mut covered, mut missed) = (0, 0);
    for (index, unit) in world.units.iter().enumerate() {
        let truth = sim
            .units
            .iter()
            .find(|t| t.site == unit.site && t.slot == unit.slot)
            .unwrap();
        // Dead in the battle, not placed by the app, and a kill of its type is open:
        // then it must be one of the suspects.
        if truth.alive
            || unit.status == Status::Destroyed
            || unit.mobile.is_some()
            || !suspects.iter().any(|s| s.0 == unit.id)
        {
            continue;
        }
        if listed.contains(&(index as u32)) {
            covered += 1;
        } else {
            missed += 1;
        }
    }
    let sizes: Vec<(String, usize)> = snapshot
        .suspects
        .iter()
        .map(|s| (s.name.clone(), s.count))
        .collect();
    println!("open kills {sizes:?}: dead units covered {covered}, missed {missed}");
    assert!(covered > 0, "the battle leaves some kills open");
    assert!(snapshot
        .suspects
        .iter()
        .all(|s| s.units.len() <= s.count && (s.units.is_empty() || s.units.len() == s.count)));
    assert!(
        missed * 10 <= covered + missed,
        "the unit really destroyed is among the suspects"
    );
}
