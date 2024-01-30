//! This is a metadata query, impl by crate `nom` or `pest`
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "../pest/query.pest"]
struct QueryParser;

fn query(q: String) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_test() {
        query("videos in 823946488".to_string());
    }
}
