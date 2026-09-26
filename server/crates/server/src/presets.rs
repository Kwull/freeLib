//! Default device presets: the shared devices a fresh install starts with, and the conversion
//! options tuned for each reader (docs/web/DEVICES.md explains the choices).
//!
//! Seeded devices remember their preset (`device.preset`) and its version. When a newer
//! server ships better defaults ([`PRESET_VERSION`]), [`upgrade`] applies them to every seeded
//! device whose conversion options nobody changed (`device.customized = 0`); a device whose
//! options an administrator edited is left alone.

use freelib_fb2conv::{ConvertOptions, CreateCover, Footnotes, Hyphenate, TocPlacement};
use rusqlite::{Connection, params};

use crate::db::{self, Device};
use crate::error::ApiResult;

/// Bumped whenever [`options`] changes; seeded devices at an older version are upgraded.
pub const PRESET_VERSION: i64 = 1;

pub struct Preset {
    pub key: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub format: &'static str,
    pub target: Option<&'static str>,
}

/// The shared devices of a fresh install, in their default order.
pub const PRESETS: [Preset; 6] = [
    Preset {
        key: "kindle-email",
        name: "Kindle",
        kind: "email",
        format: "epub",
        target: None,
    },
    Preset {
        key: "kindle-usb",
        name: "Kindle (USB)",
        kind: "download",
        format: "azw3",
        target: None,
    },
    Preset {
        key: "apple-books",
        name: "Apple Books",
        kind: "download",
        format: "epub",
        target: None,
    },
    Preset {
        key: "kobo",
        name: "Kobo",
        kind: "download",
        format: "kepub",
        target: None,
    },
    Preset {
        key: "server-folder",
        name: "Server folder",
        kind: "folder",
        format: "epub",
        target: Some(""),
    },
    Preset {
        key: "original",
        name: "Original",
        kind: "download",
        format: "original",
        target: None,
    },
];

/// Conversion options of a preset (see docs/web/DEVICES.md for the reasoning).
pub fn options(key: &str) -> ConvertOptions {
    let base = ConvertOptions {
        hyphenate: Hyphenate::Soft,
        footnotes: Footnotes::Popup,
        drop_caps: false,
        break_after_chapter: true,
        toc_placement: TocPlacement::End,
        create_cover: CreateCover::Missing,
        cover_label: None,
        join_series: false,
        transliterate: false,
        annotation: true,
        font_family: None,
        user_css: None,
    };
    match key {
        // Send to Kindle converts the EPUB itself: aside footnotes become Kindle pop-ups, soft
        // hyphens are kept (Kindle does not hyphenate Russian on its own), no embedded fonts
        // (Kindle's own fonts and its font menu work best, and fonts only add size)
        "kindle-email" => base,
        // Calibre's AZW3: the same, Calibre keeps aside notes as pop-up footnotes
        "kindle-usb" => base,
        // Apple Books: EPUB 3 pop-up notes; soft hyphens plus CSS hyphens so Apple's own
        // dictionaries hyphenate other languages too; publisher fonts stay optional
        "apple-books" => ConvertOptions {
            hyphenate: Hyphenate::Full,
            ..base
        },
        // Kobo: KEPUB (Kobo's own renderer: pop-up notes, reading stats); hyphenation dictionaries
        // on Kobo are English-only, so soft hyphens + CSS hyphens
        "kobo" => ConvertOptions {
            hyphenate: Hyphenate::Full,
            ..base
        },
        // generic EPUB for any reader app (KOReader, Moon+, PocketBook): end notes with back
        // links work everywhere
        "server-folder" => ConvertOptions {
            footnotes: Footnotes::End,
            ..base
        },
        // "Original" is never converted; keep neutral defaults
        _ => ConvertOptions::default(),
    }
}

pub fn find(key: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.key == key)
}

/// Seeds the shared default devices once (API.md "Devices and sending").
pub fn seed(c: &Connection) -> ApiResult<()> {
    if db::get_setting_raw(c, "devices_seeded")?.is_some() {
        return Ok(());
    }
    for p in &PRESETS {
        let d = Device {
            id: 0,
            name: p.name.into(),
            kind: p.kind.into(),
            format: p.format.into(),
            target: p.target.map(String::from),
            file_name: db::default_file_name(),
            shared: true,
            options: options(p.key),
            user_id: None,
            preset: Some(p.key.into()),
        };
        let id = db::save_device(c, &d)?;
        c.execute(
            "UPDATE device SET preset_version=?1 WHERE id=?2",
            params![PRESET_VERSION, id],
        )?;
    }
    db::put_setting_raw(c, "devices_seeded", "true")?;
    Ok(())
}

/// Brings seeded devices to the current presets:
/// * devices seeded by older servers (no `preset` recorded yet) are recognised by their seeded
///   name, kind, format and untouched default options;
/// * seeded devices with untouched options at an older [`PRESET_VERSION`] get the new options.
///
/// Returns the number of devices updated.
pub fn upgrade(c: &Connection) -> ApiResult<usize> {
    let mut n = 0;
    // (id, name, kind, format, file name, options)
    type Row = (i64, String, String, String, String, String);
    let legacy: Vec<Row> = {
        let mut st = c.prepare(
            "SELECT id, name, kind, format, file_name, options FROM device \
             WHERE user_id IS NULL AND preset IS NULL AND customized=0",
        )?;
        st.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<Result<_, _>>()?
    };
    for (id, name, kind, format, file_name, opts) in legacy {
        let Some(p) = PRESETS
            .iter()
            .find(|p| p.name == name && p.kind == kind && p.format == format)
        else {
            continue;
        };
        let untouched = serde_json::from_str::<ConvertOptions>(&opts)
            .is_ok_and(|o| o == ConvertOptions::default())
            && file_name == db::default_file_name();
        if untouched {
            c.execute(
                "UPDATE device SET preset=?1, preset_version=0 WHERE id=?2",
                params![p.key, id],
            )?;
        } else {
            // an edited legacy device: remember its preset, never touch its options
            c.execute(
                "UPDATE device SET preset=?1, customized=1 WHERE id=?2",
                params![p.key, id],
            )?;
        }
    }
    let outdated: Vec<(i64, String)> = {
        let mut st = c.prepare(
            "SELECT id, preset FROM device WHERE preset IS NOT NULL AND customized=0 AND preset_version<?1",
        )?;
        st.query_map([PRESET_VERSION], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    for (id, key) in outdated {
        if find(&key).is_none() {
            continue;
        }
        let o = serde_json::to_string(&options(&key)).unwrap_or_else(|_| "{}".into());
        c.execute(
            "UPDATE device SET options=?1, preset_version=?2 WHERE id=?3",
            params![o, PRESET_VERSION, id],
        )?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = freelib_catalog::open_app_db(std::path::Path::new(":memory:")).unwrap();
        rusqlite::vtab::array::load_module(&c).unwrap();
        c
    }

    #[test]
    fn fresh_install_gets_tuned_presets() {
        let c = db();
        seed(&c).unwrap();
        let devs = db::list_devices(&c, 1).unwrap();
        assert_eq!(devs.len(), 6);
        let kindle = &devs[0];
        assert_eq!(kindle.preset.as_deref(), Some("kindle-email"));
        assert_eq!(kindle.options.footnotes, Footnotes::Popup);
        assert_eq!(kindle.options.hyphenate, Hyphenate::Soft);
        assert!(kindle.options.font_family.is_none());
        assert_eq!(devs[2].options.hyphenate, Hyphenate::Full);
        assert_eq!(upgrade(&c).unwrap(), 0, "nothing to upgrade");
        seed(&c).unwrap();
        assert_eq!(db::list_devices(&c, 1).unwrap().len(), 6, "seeded once");
    }

    #[test]
    fn legacy_devices_upgraded_unless_edited() {
        let c = db();
        // what older servers seeded: default options everywhere, no preset column values
        for p in &PRESETS {
            let o = serde_json::to_string(&ConvertOptions::default()).unwrap();
            c.execute(
                "INSERT INTO device(user_id, name, kind, format, target, file_name, options) VALUES (NULL,?1,?2,?3,?4,?5,?6)",
                params![p.name, p.kind, p.format, p.target, db::default_file_name(), o],
            )
            .unwrap();
        }
        db::put_setting_raw(&c, "devices_seeded", "true").unwrap();
        // the admin changed Kobo's options and renamed Apple Books (name edits do not count,
        // but a renamed legacy device cannot be recognised any more)
        let edited = ConvertOptions {
            drop_caps: true,
            ..Default::default()
        };
        c.execute(
            "UPDATE device SET options=?1 WHERE name='Kobo'",
            [serde_json::to_string(&edited).unwrap()],
        )
        .unwrap();
        assert_eq!(upgrade(&c).unwrap(), 5);
        let devs = db::list_devices(&c, 1).unwrap();
        let by = |n: &str| devs.iter().find(|d| d.name == n).unwrap().clone();
        assert_eq!(by("Kindle").options, options("kindle-email"));
        assert_eq!(by("Kobo").options, edited, "edited options are kept");
        assert_eq!(by("Kobo").preset.as_deref(), Some("kobo"));
        assert_eq!(upgrade(&c).unwrap(), 0);
        // a later options edit through the API marks the device customized
        let mut k = by("Kindle");
        k.options.hyphenate = Hyphenate::None;
        db::save_device(&c, &k).unwrap();
        db::mark_customized_if_changed(&c, k.id, &options("kindle-email"), &k.options).unwrap();
        c.execute("UPDATE device SET preset_version=0", []).unwrap();
        assert_eq!(upgrade(&c).unwrap(), 4, "Kindle (edited) and Kobo stay");
        assert_eq!(
            db::get_device(&c, 1, k.id).unwrap().options.hyphenate,
            Hyphenate::None
        );
    }
}
