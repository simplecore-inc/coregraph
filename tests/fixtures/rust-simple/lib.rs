pub struct Config {
    pub max_retries: u32,
    pub timeout_ms: u64,
}

pub trait Processor {
    fn process(&self, input: &str) -> Result<String, ProcessError>;
    fn name(&self) -> &'static str;
}

pub enum ProcessError {
    InvalidInput(String),
    Timeout,
}

impl Config {
    pub fn new(max_retries: u32, timeout_ms: u64) -> Self {
        Config { max_retries, timeout_ms }
    }
}

pub fn default_config() -> Config {
    Config::new(3, 5000)
}
