use super::Transaction;
use crate::model::TransactionModel;

trait ModelActor {
    /// Test whether this actor owns a transaction
    fn match_transaction(t: &Transaction);

    /// Handle a transaction matching this actor
    fn log_transaction(t: &Transaction);

    /// Called when a schedule (e.g. "daily") 'ticks' over to the next period
    fn schedule_tick();
}

struct Expense {
    name: String,
    model: TransactionModel,
}

impl ModelActor for Expense {
    fn match_transaction(_: &Transaction) {
        unimplemented!();
    }

    fn log_transaction(_: &Transaction) {
        unimplemented!();
    }

    fn schedule_tick() {
        unimplemented!();
    }
}
