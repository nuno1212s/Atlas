use std::fmt::Debug;
use tracing::warn;

pub struct CircuitBreaker {
    threshold_number: usize,
    current_failures_in_row: usize,
}

const MAX_FAILURES: usize = 10;

impl CircuitBreaker {
    pub fn execute_in_circuit_breaker<F, T, E>(
        function: F,
        threshold_number: Option<usize>,
    ) -> Result<T, E>
    where
        F: FnMut() -> Result<T, E>,
        E: Debug,
    {
        let mut breaker = Self {
            threshold_number: threshold_number.unwrap_or(MAX_FAILURES),
            current_failures_in_row: 0,
        };

        breaker.execute(function)
    }

    pub fn new(threshold_number: Option<usize>) -> Self {
        Self {
            threshold_number: threshold_number.unwrap_or(MAX_FAILURES),
            current_failures_in_row: 0,
        }
    }

    fn is_open(&self) -> bool {
        self.current_failures_in_row >= self.threshold_number
    }

    pub fn execute<F, T, E>(&mut self, mut function: F) -> Result<T, E>
    where
        F: FnMut() -> Result<T, E>,
        E: Debug,
    {
        match function() {
            Ok(result) => {
                self.current_failures_in_row = 0; // Reset on success
                Ok(result)
            }
            Err(err) => {
                self.current_failures_in_row += 1;

                if self.is_open() {
                    Err(err)
                } else {
                    warn!("Error occurred, but circuit breaker is not open yet. Current failures in row: {}. Error: {:?}", self.current_failures_in_row, err);

                    self.execute(function)
                }
            }
        }
    }

    #[allow(dead_code)]
    fn reset(&mut self) {
        self.current_failures_in_row = 0;
    }
}
