use std::collections::{HashMap, HashSet};

pub struct Document {
    pub id: String,
    pub title: String,
    pub location: String,
    pub text: String,
}
pub struct Selection {
    pub indices: Vec<(usize, usize)>,
    pub method: &'static str,
}
fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.chars().count() > 2)
        .map(str::to_owned)
        .collect()
}
fn phrase_match(query: &str, phrase: &str) -> bool {
    let phrase = phrase.trim().to_lowercase();
    if phrase.is_empty() {
        return false;
    }
    query.match_indices(&phrase).any(|(i, _)| {
        let before = query[..i].chars().last();
        let after = query[i + phrase.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric())
    })
}
pub fn select(question: &str, documents: &[Document], limit: usize) -> Selection {
    let query = question.to_lowercase();
    let explicit_ids = documents
        .iter()
        .enumerate()
        .filter(|(_, d)| phrase_match(&query, &d.id))
        .map(|(i, _)| (i, 1000))
        .collect::<Vec<_>>();
    if !explicit_ids.is_empty() {
        return Selection {
            indices: explicit_ids.into_iter().take(limit).collect(),
            method: "explicit-source-id",
        };
    }
    let named_places = documents
        .iter()
        .enumerate()
        .filter(|(_, d)| d.location.trim().chars().count() > 2 && phrase_match(&query, &d.location))
        .map(|(i, _)| (i, 100))
        .collect::<Vec<_>>();
    if !named_places.is_empty() {
        return Selection {
            indices: named_places.into_iter().take(limit).collect(),
            method: "explicit-school-location",
        };
    }
    lexical(&query, documents, limit)
}
fn lexical(query: &str, documents: &[Document], limit: usize) -> Selection {
    let wanted: HashSet<_> = tokens(query).into_iter().collect();
    let indexed = documents
        .iter()
        .map(|d| {
            (
                tokens(&d.title).into_iter().collect::<HashSet<_>>(),
                tokens(&d.text).into_iter().collect::<HashSet<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let mut frequencies = HashMap::new();
    for term in &wanted {
        frequencies.insert(
            term,
            indexed
                .iter()
                .filter(|(a, b)| a.contains(term) || b.contains(term))
                .count(),
        );
    }
    let mut ranked = indexed
        .iter()
        .enumerate()
        .map(|(i, (title, text))| {
            let score = wanted
                .iter()
                .map(|t| {
                    let weight = (documents.len() + 1) * 100
                        / (frequencies.get(t).copied().unwrap_or(0) + 1);
                    (usize::from(title.contains(t)) * 3 + usize::from(text.contains(t))) * weight
                })
                .sum::<usize>();
            (i, score)
        })
        .filter(|(_, s)| *s > 0 || wanted.is_empty())
        .collect::<Vec<_>>();
    ranked.sort_by(|(a, score_a), (b, score_b)| {
        score_b
            .cmp(score_a)
            .then(documents[*a].id.cmp(&documents[*b].id))
    });
    Selection {
        indices: ranked.into_iter().take(limit).collect(),
        method: if wanted.is_empty() {
            "bounded-catalogue-sample"
        } else {
            "whole-word-lexical"
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn docs() -> Vec<Document> {
        vec![
            Document {
                id: "stapfer-2270".into(),
                title: "Jonen".into(),
                location: "Jonen".into(),
                text: "the them other thee then".into(),
            },
            Document {
                id: "stapfer-2027".into(),
                title: "Alpthal — full transcription".into(),
                location: "Alpthal".into(),
                text: "im schreiben läsen und rechnen".into(),
            },
        ]
    }
    #[test]
    fn named_place_excludes_unrelated_high_overlap() {
        let s = select(
            "Report one teaching subject from the Alpthal record",
            &docs(),
            6,
        );
        assert_eq!(s.indices, vec![(1, 100)]);
        assert_eq!(s.method, "explicit-school-location");
    }
    #[test]
    fn exact_source_id_takes_precedence() {
        let s = select("Read @stapfer-2027, not all schools", &docs(), 6);
        assert_eq!(s.indices, vec![(1, 1000)]);
    }
    #[test]
    fn locations_and_words_require_boundaries() {
        assert!(select("Alpthaler Unbekannt", &docs(), 6).indices.is_empty());
        assert!(select("oth", &docs(), 6).indices.is_empty());
    }
    #[test]
    fn two_named_places_support_comparison() {
        assert_eq!(
            select("Compare Alpthal and Jonen", &docs(), 6)
                .indices
                .len(),
            2
        );
    }
}
