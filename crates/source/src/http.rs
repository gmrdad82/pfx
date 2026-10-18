use std::io::Write;
use std::time::Duration;

use crate::{Headers, Net, USER_AGENT, shown};

pub struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Http {
        Http::new()
    }
}

impl Http {
    pub fn new() -> Http {
        let config = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .https_only(true)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .build();
        Http {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Net for Http {
    fn fetch(&self, url: &str, headers: &Headers, sink: &mut dyn Write) -> Result<u64, String> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let response = request.call().map_err(|e| match e {
            ureq::Error::StatusCode(code) => format!("{}: HTTP {code}", shown(url)),
            other => format!("{}: {other}", shown(url)),
        })?;
        let mut body = response.into_body().into_reader();
        std::io::copy(&mut body, sink).map_err(|e| format!("{}: {e}", shown(url)))
    }
}
