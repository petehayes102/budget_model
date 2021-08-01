mod contribution;
mod datelike_ext;
mod frequency;
mod transaction;

use rust_decimal::Decimal;
pub use transaction::Transaction;

// This represents the number of decimal places that a currency can validly express.
// @todo Support the full range of currency precisions specified in ISO 4217.
const CURRENCY_PRECISION: u32 = 2;

// struct Transaction {
//     amount: f32,
//     account_from: String,
//     account_to: String,
//     date: String,
//     description: Option<String>,
// }

/// Matches one or more transactions
#[derive(Default)]
pub struct TransactionMatcher {
    category: Option<String>,
    description: Option<Vec<String>>,
}

/// The value of a modelled transaction
pub enum TransactionValue {
    Fixed(Decimal),
    Variable(Decimal, Decimal), // Lower bound, upper bound
}

impl TransactionMatcher {
    pub fn with_category<S: Into<String>>(&mut self, category: S) -> &mut Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_description<S: Into<String>>(&mut self, description: S) -> &mut Self {
        self.description
            .get_or_insert(Vec::new())
            .push(description.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_matcher_with_category_and_descriptions() {
        let mut matcher = TransactionMatcher::default();
        matcher
            .with_category("abc")
            .with_description("def")
            .with_description("ghi");
        assert_eq!(matcher.category, Some("abc".into()));
        assert_eq!(matcher.description, Some(vec!["def".into(), "ghi".into()]));
    }
}
