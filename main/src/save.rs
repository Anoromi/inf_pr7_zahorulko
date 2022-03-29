use std::io::Error;

use async_trait::async_trait;
use save::save::VariableSave;
use tokio::{io::{BufWriter, BufReader}, fs::File};
