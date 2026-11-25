//! Reads the game's local web interface.
//!
//! War Thunder serves a small read-only web page on `127.0.0.1:8111` while
//! it runs (it is what the in-game "browser map" uses). This module only
//! issues GET requests to that fixed address. It never writes to the game,
//! reads its memory or touches its files.

use crate::telemetry::{self, Frame};
use std::io::Read;
use std::time::Duration;

pub const GAME_ADDRESS: &str = "http://127.0.0.1:8111";
const MAX_BODY: u64 = 4 * 1024 * 1024;

pub struct Link {
    agent: ureq::Agent,
    base: String,
    last_dmg: i64,
    tick: u64,
    /// Last map generation, to restart the kill-feed cursor on a new battle.
    generation: Option<i64>,
}

impl Link {
    pub fn new() -> Link {
        Link::with_base(GAME_ADDRESS)
    }

    /// `base` exists for tests and the simulator; the app always uses
    /// [`GAME_ADDRESS`].
    pub fn with_base(base: &str) -> Link {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(400))
            .timeout_read(Duration::from_millis(1200))
            .timeout_write(Duration::from_millis(400))
            .redirects(0)
            .build();
        Link {
            agent,
            base: base.trim_end_matches('/').to_string(),
            last_dmg: 0,
            tick: 0,
            generation: None,
        }
    }

    fn get(&self, path: &str) -> Option<String> {
        let response = self
            .agent
            .get(&format!("{}{}", self.base, path))
            .call()
            .ok()?;
        let mut body = String::new();
        response
            .into_reader()
            .take(MAX_BODY)
            .read_to_string(&mut body)
            .ok()?;
        Some(body)
    }

    /// One page as text, unparsed (used by the battle recorder).
    pub fn raw(&self, path: &str) -> Option<String> {
        self.get(path)
    }

    /// Raw bytes of the game's own map picture, for maps the app has no art for.
    pub fn map_image(&self) -> Option<Vec<u8>> {
        let response = self
            .agent
            .get(&format!("{}/map.img", self.base))
            .call()
            .ok()?;
        let mut body = Vec::new();
        response
            .into_reader()
            .take(16 * 1024 * 1024)
            .read_to_end(&mut body)
            .ok()?;
        if body.len() < 64 {
            return None;
        }
        Some(body)
    }

    /// One poll. Fast-changing pages are read every time, slow ones in turn.
    pub fn poll(&mut self) -> Frame {
        self.tick += 1;
        let mut frame = Frame::default();
        let Some(objects) = self.get("/map_obj.json") else {
            return frame; // game not running
        };
        frame.reachable = true;
        frame.objects = telemetry::parse_map_obj(&objects);
        if self.tick % 4 == 1 {
            frame.map_info = self
                .get("/map_info.json")
                .and_then(|t| telemetry::parse_map_info(&t));
            if let Some(Some(info)) = &frame.map_info {
                if self.generation.is_some_and(|g| g != info.generation) {
                    self.last_dmg = 0;
                }
                self.generation = Some(info.generation);
            }
        }
        if self.tick % 2 == 0 {
            if let Some(lines) = self
                .get(&format!("/hudmsg?lastEvt=0&lastDmg={}", self.last_dmg))
                .and_then(|t| telemetry::parse_hudmsg(&t))
            {
                if let Some(max) = lines.iter().map(|l| l.id).max() {
                    self.last_dmg = self.last_dmg.max(max);
                }
                frame.hud = lines;
            }
        }
        match self.tick % 8 {
            3 => {
                frame.vehicle = self
                    .get("/indicators")
                    .and_then(|t| telemetry::parse_indicators(&t))
            }
            5 => frame.flight = self.get("/state").and_then(|t| telemetry::parse_state(&t)),
            7 => {
                frame.mission_status = self
                    .get("/mission.json")
                    .and_then(|t| telemetry::parse_mission(&t))
            }
            _ => {}
        }
        frame
    }
}

impl Default for Link {
    fn default() -> Self {
        Link::new()
    }
}
