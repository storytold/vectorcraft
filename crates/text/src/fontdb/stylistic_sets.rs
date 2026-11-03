//! The font-defined OpenType stylistic sets offered by the OpenType panel.
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/features_pt>

use skrifa::raw::TableProvider;
use skrifa::raw::tables::layout::{FeatureList, FeatureParams};

use super::FontFace;
use super::names::name;

/// One `ss01`–`ss20` feature in a face. `number` is the tag's number; `name` is the font's UI
/// label when it supplies one, otherwise `Set 1` through `Set 20`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StylisticSet {
    pub number: u8,
    pub name: String,
}

/// The `ssNN` number in a feature tag, if it is one of the twenty registered sets.
fn number(tag: [u8; 4]) -> Option<usize> {
    if &tag[..2] != b"ss" || !tag[2].is_ascii_digit() || !tag[3].is_ascii_digit() {
        return None;
    }
    let n = usize::from(tag[2] - b'0') * 10 + usize::from(tag[3] - b'0');
    (1..=20).contains(&n).then_some(n)
}

/// Collect registered set records from either layout table. Fonts may carry a set in GSUB, GPOS,
/// or both. A readable custom name takes precedence over the numbered fallback.
fn collect(font: &skrifa::FontRef<'_>, features: FeatureList<'_>, labels: &mut [Option<String>; 20]) {
    for record in features.feature_records().iter().take(4096) {
        let Some(n) = number(record.feature_tag().to_be_bytes()) else { continue };
        let Ok(feature) = record.feature(features.offset_data()) else { continue };
        if feature.lookup_list_indices().is_empty() {
            continue;
        }
        let custom = feature.feature_params().and_then(Result::ok).and_then(|params| match params {
            FeatureParams::StylisticSet(p) => name(font, &[p.ui_name_id()]),
            _ => None,
        });
        let slot = &mut labels[n - 1];
        if let Some(custom) = custom {
            *slot = Some(custom);
        } else if slot.is_none() {
            *slot = Some(format!("Set {n}"));
        }
    }
}

impl FontFace {
    /// Sets this face lists in GSUB or GPOS, in number order. Damaged tables and records are
    /// skipped, and no more than 4096 feature records from either table are read.
    pub fn stylistic_sets(&self) -> Vec<StylisticSet> {
        let Some(font) = self.skrifa() else { return Vec::new() };
        let mut labels: [Option<String>; 20] = std::array::from_fn(|_| None);
        if let Ok(gsub) = font.gsub()
            && let Ok(features) = gsub.feature_list()
        {
            collect(&font, features, &mut labels);
        }
        if let Ok(gpos) = font.gpos()
            && let Ok(features) = gpos.feature_list()
        {
            collect(&font, features, &mut labels);
        }
        labels.into_iter().enumerate().filter_map(|(i, name)| Some(StylisticSet { number: u8::try_from(i + 1).ok()?, name: name? })).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrifa::MetadataProvider;
    use write_fonts::FontBuilder;
    use write_fonts::tables::gsub::{Gsub, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList};
    use write_fonts::tables::layout::{
        CoverageFormat1, Feature, FeatureList, FeatureParams as WriteParams, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord,
        StylisticSetParams,
    };
    use write_fonts::types::{GlyphId16, NameId, Tag};

    #[test]
    fn registered_set_numbers_only() {
        assert_eq!(number(*b"ss01"), Some(1));
        assert_eq!(number(*b"ss20"), Some(20));
        assert_eq!(number(*b"ss00"), None);
        assert_eq!(number(*b"ss21"), None);
        assert_eq!(number(*b"salt"), None);
    }

    #[test]
    fn face_lists_only_its_sets_with_the_fonts_name() {
        const BASE: &[u8] = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf");
        let base = skrifa::FontRef::new(BASE).unwrap();
        let gid = |c| GlyphId16::new(u16::try_from(base.charmap().map(c).unwrap().to_u32()).unwrap());
        let subst = SingleSubstFormat2::new(CoverageFormat1::new(vec![gid('a')]).into(), vec![gid('A')]);
        let lookup = SubstitutionLookup::Single(Lookup::new(Default::default(), vec![subst.into()]));
        let features = FeatureList::new(vec![
            FeatureRecord::new(Tag::new(b"ss01"), Feature::new(Some(WriteParams::StylisticSet(StylisticSetParams::new(NameId::new(256)))), vec![0])),
            FeatureRecord::new(Tag::new(b"ss03"), Feature::new(None, vec![0])),
            FeatureRecord::new(Tag::new(b"ss05"), Feature::new(None, vec![])),
        ]);
        let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0, 1, 2])), vec![]))]);
        let mut builder = FontBuilder::new();
        let names = crate::test_fonts::name_table(&[
            (1, "Stylistic Test Sans"),
            (2, "Regular"),
            (4, "Stylistic Test Sans Regular"),
            (6, "StylisticTestSans-Regular"),
            (256, "Curved Capitals"),
        ])
        .unwrap();
        builder.add_raw(Tag::new(b"name"), names);
        builder.add_table(&Gsub::new(scripts, features, SubstitutionLookupList::new(vec![lookup]))).unwrap();
        for record in base.table_directory.table_records() {
            let tag = Tag::new(&record.tag().to_be_bytes());
            if !builder.contains(tag) {
                builder.add_raw(tag, base.table_data(record.tag()).unwrap().as_bytes());
            }
        }
        let db = super::super::FontDb::with_font_dirs(vec![]);
        assert_eq!(db.add_font(builder.build()), 1);
        let face = db.face("Stylistic Test Sans", "Regular").unwrap();
        assert_eq!(
            face.stylistic_sets(),
            [StylisticSet { number: 1, name: "Curved Capitals".into() }, StylisticSet { number: 3, name: "Set 3".into() }]
        );
    }
}
