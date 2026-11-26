//! The font-defined OpenType stylistic sets offered by the OpenType panel.
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/features_pt>

use std::collections::HashMap;

use skrifa::raw::tables::layout::{Feature, FeatureList, FeatureParams, FeatureVariations};
use skrifa::raw::types::Tag;
use skrifa::raw::{ResolveOffset, TableProvider};

use super::FontFace;
use super::names::name;

/// One `ss01`–`ss20` feature in a face. `number` is the tag's number; `name` is the font's UI
/// label when it supplies one, and `None` otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StylisticSet {
    pub number: u8,
    pub name: Option<String>,
}

/// The `ssNN` number in a feature tag, if it is one of the twenty registered sets.
fn number(tag: [u8; 4]) -> Option<usize> {
    if &tag[..2] != b"ss" || !tag[2].is_ascii_digit() || !tag[3].is_ascii_digit() {
        return None;
    }
    let n = usize::from(tag[2] - b'0') * 10 + usize::from(tag[3] - b'0');
    (1..=20).contains(&n).then_some(n)
}

/// The features that FeatureVariations substitutions replace with a feature that has lookups,
/// keyed by feature index. A variable font can leave a set's default feature empty and supply its
/// lookups only at some variation locations. `tags` holds each feature's tag, which a substituted
/// feature needs to read its parameters. At most 4096 variation records and 4096 substitutions are
/// read.
fn variation_features<'a>(variations: Option<FeatureVariations<'a>>, tags: &[Tag]) -> HashMap<usize, Feature<'a>> {
    let mut found = HashMap::new();
    let Some(variations) = variations else { return found };
    let mut budget = 4096_usize;
    for record in variations.feature_variation_records().iter().take(4096) {
        if let Some(Ok(substitution)) = record.feature_table_substitution(variations.offset_data()) {
            for replacement in substitution.substitutions() {
                if budget == 0 {
                    return found;
                }
                budget -= 1;
                let index = usize::from(replacement.feature_index());
                let Some(&tag) = tags.get(index) else { continue };
                if let Ok(alternate) = replacement.alternate_feature_offset().resolve_with_args::<Feature>(substitution.offset_data(), tag)
                    && !alternate.lookup_list_indices().is_empty()
                {
                    found.entry(index).or_insert(alternate);
                }
            }
        }
        // Only the first record whose conditions match applies, so records after one without
        // conditions are never used.
        let unconditional = match record.condition_set(variations.offset_data()) {
            None => true,
            Some(Ok(conditions)) => conditions.condition_count() == 0,
            Some(Err(_)) => false,
        };
        if unconditional {
            break;
        }
    }
    found
}

/// Collect registered set records from either layout table. Fonts may carry a set in GSUB, GPOS,
/// or both. A record with no lookups counts only when a FeatureVariations substitution gives it
/// some. `labels[n - 1]` is `Some` once set `n` is found, holding its readable custom name if any
/// record supplies one.
fn collect(
    font: &skrifa::FontRef<'_>,
    features: FeatureList<'_>,
    variations: Option<FeatureVariations<'_>>,
    labels: &mut [Option<Option<String>>; 20],
) {
    let tags: Vec<Tag> = features.feature_records().iter().take(4096).map(|record| record.feature_tag()).collect();
    let substituted = variation_features(variations, &tags);
    for (index, record) in features.feature_records().iter().enumerate().take(4096) {
        let Some(n) = number(record.feature_tag().to_be_bytes()) else { continue };
        let Ok(default) = record.feature(features.offset_data()) else { continue };
        let label = |feature: &Feature<'_>| {
            feature.feature_params().and_then(Result::ok).and_then(|params| match params {
                FeatureParams::StylisticSet(p) => name(font, &[p.ui_name_id()]),
                _ => None,
            })
        };
        let alternate = substituted.get(&index);
        if default.lookup_list_indices().is_empty() && alternate.is_none() {
            continue;
        }
        let custom = label(&default).or_else(|| alternate.and_then(label));
        let slot = &mut labels[n - 1];
        if custom.is_some() {
            *slot = Some(custom);
        } else if slot.is_none() {
            *slot = Some(None);
        }
    }
}

impl FontFace {
    /// Sets this face lists in GSUB or GPOS, in number order. Damaged tables and records are
    /// skipped, and no more than 4096 feature records from either table are read.
    pub fn stylistic_sets(&self) -> Vec<StylisticSet> {
        let Some(font) = self.skrifa() else { return Vec::new() };
        let mut labels: [Option<Option<String>>; 20] = std::array::from_fn(|_| None);
        if let Ok(gsub) = font.gsub()
            && let Ok(features) = gsub.feature_list()
        {
            collect(&font, features, gsub.feature_variations().and_then(Result::ok), &mut labels);
        }
        if let Ok(gpos) = font.gpos()
            && let Ok(features) = gpos.feature_list()
        {
            collect(&font, features, gpos.feature_variations().and_then(Result::ok), &mut labels);
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
        Condition, ConditionFormat1, ConditionSet, CoverageFormat1, Feature, FeatureList, FeatureParams as WriteParams, FeatureRecord,
        FeatureTableSubstitution, FeatureTableSubstitutionRecord, FeatureVariationRecord, FeatureVariations as WriteVariations, LangSys, Lookup,
        Script, ScriptList, ScriptRecord, StylisticSetParams,
    };
    use write_fonts::types::{F2Dot14, GlyphId16, NameId, Tag};

    #[test]
    fn registered_set_numbers_only() {
        assert_eq!(number(*b"ss01"), Some(1));
        assert_eq!(number(*b"ss20"), Some(20));
        assert_eq!(number(*b"ss00"), None);
        assert_eq!(number(*b"ss21"), None);
        assert_eq!(number(*b"salt"), None);
    }

    fn face_with(features: FeatureList, variations: Option<WriteVariations>) -> std::sync::Arc<FontFace> {
        const BASE: &[u8] = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf");
        let base = skrifa::FontRef::new(BASE).unwrap();
        let gid = |c| GlyphId16::new(u16::try_from(base.charmap().map(c).unwrap().to_u32()).unwrap());
        let subst = SingleSubstFormat2::new(CoverageFormat1::new(vec![gid('a')]).into(), vec![gid('A')]);
        let lookup = SubstitutionLookup::Single(Lookup::new(Default::default(), vec![subst.into()]));
        let indices = (0..u16::try_from(features.feature_records.len()).unwrap()).collect();
        let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(indices)), vec![]))]);
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
        let mut gsub = Gsub::new(scripts, features, SubstitutionLookupList::new(vec![lookup]));
        gsub.feature_variations = variations.into();
        builder.add_table(&gsub).unwrap();
        for record in base.table_directory.table_records() {
            let tag = Tag::new(&record.tag().to_be_bytes());
            if !builder.contains(tag) {
                builder.add_raw(tag, base.table_data(record.tag()).unwrap().as_bytes());
            }
        }
        let db = super::super::FontDb::with_font_dirs(vec![]);
        assert_eq!(db.add_font(builder.build()), 1);
        db.face("Stylistic Test Sans", "Regular").unwrap()
    }

    fn set_params() -> Option<WriteParams> {
        Some(WriteParams::StylisticSet(StylisticSetParams::new(NameId::new(256))))
    }

    #[test]
    fn face_lists_only_its_sets_with_the_fonts_name() {
        let features = FeatureList::new(vec![
            FeatureRecord::new(Tag::new(b"ss01"), Feature::new(set_params(), vec![0])),
            FeatureRecord::new(Tag::new(b"ss03"), Feature::new(None, vec![0])),
            FeatureRecord::new(Tag::new(b"ss05"), Feature::new(None, vec![])),
        ]);
        let face = face_with(features, None);
        assert_eq!(face.stylistic_sets(), [StylisticSet { number: 1, name: Some("Curved Capitals".into()) }, StylisticSet { number: 3, name: None }]);
    }

    #[test]
    fn a_set_whose_lookups_come_from_feature_variations_is_listed() {
        let features = FeatureList::new(vec![
            FeatureRecord::new(Tag::new(b"ss02"), Feature::new(None, vec![0])),
            FeatureRecord::new(Tag::new(b"ss05"), Feature::new(None, vec![])),
            FeatureRecord::new(Tag::new(b"ss07"), Feature::new(None, vec![])),
            FeatureRecord::new(Tag::new(b"ss09"), Feature::new(set_params(), vec![])),
        ]);
        let condition =
            ConditionSet::new(vec![Condition::Format1AxisRange(ConditionFormat1::new(0, F2Dot14::from_f32(0.5), F2Dot14::from_f32(1.0)))]);
        let conditional = FeatureTableSubstitution::new(vec![
            FeatureTableSubstitutionRecord::new(1, Feature::new(set_params(), vec![0])),
            FeatureTableSubstitutionRecord::new(2, Feature::new(None, vec![])),
        ]);
        let always = FeatureTableSubstitution::new(vec![FeatureTableSubstitutionRecord::new(3, Feature::new(None, vec![0]))]);
        let unreachable = FeatureTableSubstitution::new(vec![FeatureTableSubstitutionRecord::new(2, Feature::new(None, vec![0]))]);
        let variations = WriteVariations::new(vec![
            FeatureVariationRecord::new(Some(condition), Some(conditional)),
            FeatureVariationRecord::new(None, Some(always)),
            FeatureVariationRecord::new(None, Some(unreachable)),
        ]);
        let face = face_with(features, Some(variations));
        assert_eq!(
            face.stylistic_sets(),
            [
                StylisticSet { number: 2, name: None },
                StylisticSet { number: 5, name: Some("Curved Capitals".into()) },
                StylisticSet { number: 9, name: Some("Curved Capitals".into()) },
            ]
        );
    }
}
