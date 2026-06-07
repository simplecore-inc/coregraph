pub struct Server {
    port: u16,
}

impl Server {
    pub fn new(port: u16) -> Self {
        Server { port }
    }

    pub fn start(&self) -> Result<(), String> {
        if self.port == 0 {
            return Err("invalid port".into());
        }
        Ok(())
    }
}

pub fn make_server(port: u16) -> Server {
    Server::new(port)
}
