use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

pub fn fuzzy_match(query: &str, target: &str) -> Option<i64> {
    let matcher = SkimMatcherV2::default();
    matcher.fuzzy_match(target, query)
}

pub fn format_app_name(name: &str) -> String {
    name.replace(".desktop", "")
}

