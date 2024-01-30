//! This is a metadata query, impl by crate `nom` or `pest`
use crate::proto::data::Meta;
use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "../pest/query.pest"]
struct QueryParser;

impl Meta {
    pub(crate) fn query(&self, q: String) {
        match QueryParser::parse(Rule::query, &q) {
            Ok(mut pairs) => {
                let pairs = pairs.next().unwrap().into_inner();
                for p in pairs {
                    match p.as_rule() {
                        Rule::target => todo!(),
                        Rule::relation => todo!(),
                        Rule::id => todo!(),
                        _ => {}
                    }
                }
            }
            Err(e) => {
                #[cfg(test)]
                println!("{e}");
                tracing::error!("{}", e);
            }
        }
    }
}

#[derive(Default)]
struct Query {
    target: Target,
    relation: Relation,
    id: String,
}

#[derive(Default)]
enum Target {
    #[default]
    Video,
    List,
}

#[derive(Default)]
enum Relation {
    #[default]
    Eq,
    In,
}

#[cfg(test)]
mod tests {
    use crate::meta::meta;

    #[test]
    fn query_test() {
        meta().query("videos in 823946488".to_string());
    }
}
