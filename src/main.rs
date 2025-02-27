use std::io::Write;
use std::fmt::Display;
use std::path::PathBuf;
use clap::value_parser;
use clap::Command;
use std::path::Path;
use std::sync::Arc;
use serde::Deserialize;
use std::collections::HashMap;

fn main() {
    // parse command line options
    let cli = Command::new(env!("CARGO_BIN_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .long_about("Rezolus provides high-resolution systems performance telemetry.")
        .subcommand_negates_reqs(true)
        .arg(
            clap::Arg::new("CONFIG")
                .help("Pinball configuration file")
                .value_parser(value_parser!(PathBuf))
                .action(clap::ArgAction::Set)
                .required(true)
                .index(1),
        )
        .arg(
            clap::Arg::new("PROFILE")
                .help("Pinball profile name")
                .value_parser(value_parser!(String))
                .action(clap::ArgAction::Set)
                .required(true)
                .index(2),
        )
        .get_matches();

    let config_path: PathBuf = cli.get_one::<PathBuf>("CONFIG").unwrap().to_path_buf();
    let profile_name: String = cli.get_one::<String>("PROFILE").unwrap().to_string();

    let config: Arc<Config> = {
        println!("loading config: {:?}", config_path);
        match Config::load(&config_path) {
            Ok(c) => c.into(),
            Err(error) => {
                eprintln!("error loading config file: {:?}\n{error}", config_path);
                std::process::exit(1);
            }
        }
    };

    let profile = config.profile(&profile_name).unwrap_or_else(|| {
        eprintln!("profile: {profile_name} was not found in the config: {:?}", config_path);
        std::process::exit(1);
    });

    for nic in &profile.network_interface {
        nic.configure();
    }

}

#[derive(Deserialize, Default)]
pub struct Config {
    profile: Vec<Profile>,
}

impl Config {
    pub fn load(path: &dyn AsRef<Path>) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| {
                eprintln!("unable to open config file: {e}");
                std::process::exit(1);
            })
            .unwrap();

        let config: Config = toml::from_str(&content)
            .map_err(|e| {
                eprintln!("failed to parse config file: {e}");
                std::process::exit(1);
            })
            .unwrap();

        Ok(config)
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profile.iter().find(|&profile| profile.name == name)
    }
}

#[derive(Deserialize, Default)]
pub struct Profile {
    name: String,
    network_interface: Vec<NetworkInterface>,
}

#[derive(Deserialize, Default)]
pub struct NetworkInterface {
    name: String,
    queues: NetworkQueues,
    irqs: HashMap<String, String>,
}

impl NetworkInterface {
    pub fn configure(&self) {
        println!("configuring IRQs for: {}", self.name);
        println!("setting queues to: {}", self.queues);
        self.configure_queues();
        println!("setting IRQ affinity");
        self.set_irq_affinity();
    }

    fn configure_queues(&self) {
        self.queues.apply(&self.name);
    }

    fn set_irq_affinity(&self) {
        for (irq, affinity) in self.irqs.iter() {
            let irq: u32 = irq.parse().expect("failed to parse");

            // validate the affinity list doesn't contain anything funky
            assert!(affinity.bytes().all(|b| b.is_ascii_digit() || b == b"-"[0] || b == b","[0]));

            for i in 0..5 {
                if let Ok(mut f) = std::fs::File::options().write(true).truncate(true).create(false).open(format!("/proc/irq/{irq}/smp_affinity_list")) {
                    if f.write_all(affinity.as_bytes()).is_ok() {
                        break;
                    }
                }

                std::thread::sleep(core::time::Duration::from_millis(100));

                if i == 4 {
                    eprintln!("failed to set irq: {irq} smp affinity list: {affinity}");
                    std::process::exit(1);
                }
            }
        }
    }
}

#[derive(Deserialize, Default)]
pub struct NetworkQueues {
    transmit: Option<usize>,
    receive: Option<usize>,
    combined: Option<usize>,
}

impl NetworkQueues {
    // Apply the queue configuration to the specified network interface
    pub fn apply(&self, nic: &str) {
        // we're about to use this in a command, make sure there's no way to
        // escape and run something else
        assert!(nic.bytes().all(|b| b.is_ascii_alphanumeric()));

        std::process::Command::new("/usr/sbin/ethtool")
            .arg("-L")
            .arg(nic)
            .args(self.args())
            .output()
            .expect("failed to execute process");
    }

    // Turn the config into a set of args to pass to ethtool
    fn args(&self) -> Vec<String> {
        let mut r = Vec::new();
        
        if let Some(tx) = self.transmit {
            r.push("tx".into());
            r.push(format!("{tx}"));
        }

        if let Some(rx) = self.receive {
            r.push("rx".into());
            r.push(format!("{rx}"));
        }

        if let Some(combined) = self.combined {
            r.push("combined".into());
            r.push(format!("{combined}"));
        }

        r
    }
}

impl Display for NetworkQueues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        let s = self.args().join(" ");
        write!(f, "{s}")
    }
}