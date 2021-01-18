//! A standalone crate to calculate star ratings and performance points for all [osu!](https://osu.ppy.sh/home) gamemodes.
//!
//! Conversions are generally not supported.
//!
//! ### Usage
//! ```rust,no_run
//! use std::fs::File;
//! use rosu_pp::{Beatmap, BeatmapExt, GameMode, OsuPP, TaikoPP};
//!
//! let file = match File::open("/path/to/file.osu") {
//!     Ok(file) => file,
//!     Err(why) => panic!("Could not open file: {}", why),
//! };
//!
//! // Parse the map yourself
//! let map = match Beatmap::parse(file) {
//!     Ok(map) => map,
//!     Err(why) => panic!("Error while parsing map: {}", why),
//! };
//!
//! // The different modes make things annoying because their
//! // pp calculations require different parameters.
//! // For now, you will have to match on the mode yourself
//! // to be able to set all options for pp calculation.
//! match map.mode {
//!     GameMode::STD => {
//!         let result = OsuPP::new(&map)
//!             .mods(24) // HDHR
//!             .combo(1234)
//!             .misses(2)
//!             .accuracy(99.2)
//!             // `no_leniency` is the suggested default
//!             .calculate(rosu_pp::osu::no_leniency::stars);
//!
//!         println!("PP: {}", result.pp());
//!
//!         // If you intend to reuse the current map-mod combination,
//!         // make use of the previous result!
//!         // If attributes are given, then stars & co don't have to be recalculated.
//!         let next_result = OsuPP::new(&map)
//!             .mods(24) // HDHR
//!             .attributes(result)
//!             .combo(543)
//!             .misses(5)
//!             .n50(3)
//!             .accuracy(97.5)
//!             .calculate(rosu_pp::osu::no_leniency::stars);
//!
//!         println!("Next PP: {}", next_result.pp());
//!     },
//!     GameMode::TKO => {
//!         let result = TaikoPP::new(&map)
//!             .mods(64) // DT
//!             .combo(555)
//!             .misses(10)
//!             .passed_objects(600)
//!             .accuracy(95.12345)
//!             .calculate();
//!
//!         println!("Stars: {} | PP: {}", result.stars(), result.pp());
//!     }
//!     GameMode::MNA | GameMode::CTB => panic!("do your thing"),
//! }
//!
//! // If all you want is the map's stars or max pp,
//! // you can make use of the BeatmapExt trait.
//! let stars = map.stars(16, None).stars(); // HR
//! let max_pp = map.max_pp(16).pp();
//!
//! println!("Stars: {} | Max PP: {}", stars, max_pp);
//! ```
//!
//! ### osu!standard versions
//! - `all_included`: WIP
//! - `no_leniency`: The positional offset of notes created by stack leniency is not considered. This means the jump distance inbetween notes might be slightly off, resulting in small inaccuracies. Since calculating these offsets is relatively expensive though, this version is considerably faster than `all_included`.
//! - `no_slider_no_leniency` (i.e. oppai): In addtion to not considering the positional offset caused by stack leniency, slider paths are also ignored. This means the travel distance of notes is completely omitted which may cause further inaccuracies. Since the slider paths don't have to be computed though, it should generally be faster than `no_leniency`.
//!
//! ### Roadmap
//! - osu sr versions
//!   - [ ] all included
//!   - [x] no_leniency
//!   - [x] no_sliders_no_leniency (i.e. oppai)
//! - [x] taiko sr
//! - [x] ctb sr
//! - [x] mania sr
//! ---
//! - [x] osu pp
//! - [x] taiko pp
//! - [x] ctb pp
//! - [x] mania pp
//! ---
//! - [x] refactoring
//! - [ ] benchmarking

pub mod fruits;
pub mod mania;
pub mod osu;
pub mod parse;
pub mod taiko;

mod curve;
mod math_util;
mod mods;

pub use fruits::FruitsPP;
pub use mania::ManiaPP;
pub use osu::OsuPP;
pub use taiko::TaikoPP;

pub use mods::Mods;
pub use parse::{
    Beatmap, BeatmapAttributes, DifficultyPoint, GameMode, HitObject, HitObjectKind, HitSound,
    PathType, Pos2, TimingPoint,
};

pub trait BeatmapExt {
    /// Calculate the stars and other attributes of a beatmap which are required for pp calculation.
    ///
    /// For osu!standard maps, the `no_leniency` version will be used.
    fn stars(&self, mods: impl Mods, passed_objects: Option<usize>) -> StarResult;

    /// Calculate the max pp of a beatmap if that is all you want.
    ///
    /// For osu!standard maps, the `no_leniency` version will be used.
    ///
    /// If you seek more fine-tuning and options you need to match on the map's
    /// mode and use the mode's corresponding calculator, e.g. [`TaikoPP`](crate::TaikoPP) for taiko.
    fn max_pp(&self, mods: u32) -> PpResult;

    /// Calculate the strains of a map.
    /// This essentially performs the same calculation as a `stars` function but
    /// instead of evaluating the final strains, they are just returned as is.
    ///
    /// Suitable to plot the difficulty of a map over time.
    ///
    /// For osu!standard maps, the `no_leniency` version will be used.
    fn strains(&self, mods: impl Mods) -> Strains;
}

impl BeatmapExt for Beatmap {
    fn stars(&self, mods: impl Mods, passed_objects: Option<usize>) -> StarResult {
        match self.mode {
            GameMode::STD => osu::no_leniency::stars(self, mods, passed_objects),
            GameMode::MNA => mania::stars(self, mods, passed_objects),
            GameMode::TKO => taiko::stars(self, mods, passed_objects),
            GameMode::CTB => fruits::stars(self, mods, passed_objects),
        }
    }
    fn max_pp(&self, mods: u32) -> PpResult {
        match self.mode {
            GameMode::STD => OsuPP::new(self)
                .mods(mods)
                .calculate(osu::no_leniency::stars),
            GameMode::MNA => ManiaPP::new(self).mods(mods).calculate(),
            GameMode::TKO => TaikoPP::new(self).mods(mods).calculate(),
            GameMode::CTB => FruitsPP::new(self).mods(mods).calculate(),
        }
    }
    fn strains(&self, mods: impl Mods) -> Strains {
        match self.mode {
            GameMode::STD => osu::no_leniency::strains(self, mods),
            GameMode::MNA => mania::strains(self, mods),
            GameMode::TKO => taiko::strains(self, mods),
            GameMode::CTB => fruits::strains(self, mods),
        }
    }
}

/// The result of calculating the strains on a map.
/// Suitable to plot the difficulty of a map over time.
///
/// `strains` will be the summed strains for each skill of the map's mode.
///
/// `section_length` is the time in ms inbetween two strains.
#[derive(Clone, Debug, Default)]
pub struct Strains {
    pub section_length: f32,
    pub strains: Vec<f32>,
}

/// Basic enum containing the result of a star calculation based on the mode.
#[derive(Clone, Debug)]
pub enum StarResult {
    Fruits {
        attributes: fruits::DifficultyAttributes,
    },
    Mania {
        stars: f32,
    },
    Osu {
        attributes: osu::DifficultyAttributes,
    },
    Taiko {
        stars: f32,
    },
}

impl StarResult {
    /// The final star value.
    #[inline]
    pub fn stars(&self) -> f32 {
        match self {
            Self::Fruits { attributes, .. } => attributes.stars,
            Self::Mania { stars } => *stars,
            Self::Osu { attributes, .. } => attributes.stars,
            Self::Taiko { stars } => *stars,
        }
    }
}

/// Basic struct containing the result of a PP calculation.
#[derive(Clone, Debug)]
pub struct PpResult {
    pub pp: f32,
    pub attributes: StarResult,
}

impl PpResult {
    /// The final pp value.
    #[inline]
    pub fn pp(&self) -> f32 {
        self.pp
    }

    /// The final star value.
    #[inline]
    pub fn stars(&self) -> f32 {
        self.attributes.stars()
    }
}

#[inline]
fn difficulty_range(val: f32, max: f32, avg: f32, min: f32) -> f32 {
    if val > 5.0 {
        avg + (max - avg) * (val - 5.0) / 5.0
    } else if val < 5.0 {
        avg - (avg - min) * (5.0 - val) / 5.0
    } else {
        avg
    }
}
