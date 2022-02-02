use clap::Parser as ClapParser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use std::io::Error as IoError;
use std::path::Path;
use std::sync::Arc;

use async_std::{fs::OpenOptions, io};
use tempfile::TempDir;
use tide::prelude::*;
use tide::{Body, Request, Response, StatusCode};

use handlebars::Handlebars;
use std::collections::BTreeMap;
use tide_handlebars::prelude::*;

#[derive(Clone)]
struct AyTestState {
    tempdir: Arc<TempDir>,
    registry: Handlebars<'static>,
}

impl AyTestState {
    fn try_new() -> Result<Self, IoError> {
        Ok(Self {
            tempdir: Arc::new(tempfile::tempdir()?),
            registry: Handlebars::new(),
        })
    }

    fn path(&self) -> &Path {
        self.tempdir.path()
    }
}

/// This program does something useful, but its author needs to edit this.
/// Else it will be just hanging around forever
#[derive(Debug, Clone, ClapParser, Serialize, Deserialize)]
#[clap(version = env!("GIT_VERSION"), author = "Andrew Yourtchenko <ayourtch@gmail.com>")]
struct Opts {
    /// Target hostname to do things on
    #[clap(short, long, default_value = "localhost")]
    target_host: String,

    /// Override options from this yaml/json file
    #[clap(short, long)]
    options_override: Option<String>,

    /// A level of verbosity, and can be used multiple times
    #[clap(short, long, parse(from_occurrences))]
    verbose: i32,
}

#[derive(Debug, Deserialize)]
struct Animal {
    name: String,
    legs: u16,
}

#[async_std::main]
async fn main() -> tide::Result<()> {
    let opts: Opts = Opts::parse();

    // allow to load the options, so far there is no good built-in way
    let opts = if let Some(fname) = &opts.options_override {
        if let Ok(data) = std::fs::read_to_string(&fname) {
            let res = serde_json::from_str(&data);
            if res.is_ok() {
                res.unwrap()
            } else {
                serde_yaml::from_str(&data).unwrap()
            }
        } else {
            opts
        }
    } else {
        opts
    };

    if opts.verbose > 4 {
        let data = serde_json::to_string_pretty(&opts).unwrap();
        println!("{}", data);
        println!("===========");
        let data = serde_yaml::to_string(&opts).unwrap();
        println!("{}", data);
    }

    println!("Hello, here is your options: {:#?}", &opts);

    // let mut app = tide::new();
    tide::log::start();
    let mut state = AyTestState::try_new()?;
    state
        .registry
        .register_template_file("simple.html", "./templates/simple.html")
        .unwrap();
    state
        .registry
        .register_templates_directory("", "./templates/")
        .unwrap();

    let mut app = tide::with_state(state);
    app.at("/orders/shoes").post(order_shoes);
    app.at("/file/:file").put(upload_file).get(download_file);
    app.at("/static/").serve_dir("static/")?;
    app.at("/:name")
        .get(|req: tide::Request<AyTestState>| async move {
            let hb = &req.state().registry;
            let name: String = req.param("name")?.into();
            let mut data0 = BTreeMap::new();
            let mut names: Vec<String> = vec![];
            names.push(name.clone());
            names.push("staticname".to_string());
            data0.insert("name".to_string(), names);
            Ok(hb.render(&name, &data0)?)
        });
    app.listen("127.0.0.1:8080").await?;
    Ok(())
}

async fn upload_file(mut req: Request<AyTestState>) -> tide::Result<serde_json::Value> {
    let path = req.param("file")?;
    let fs_path = req.state().path().join(path);

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&fs_path)
        .await?;

    let bytes_written = io::copy(req, file).await?;

    tide::log::info!("file written", {
        bytes: bytes_written,
        path: fs_path.canonicalize()?.to_str()
    });

    Ok(json!({ "bytes": bytes_written }))
}
async fn download_file(mut req: Request<AyTestState>) -> tide::Result {
    let path = req.param("file")?;
    let fs_path = req.state().path().join(path);

    if let Ok(body) = Body::from_file(fs_path).await {
        Ok(body.into())
    } else {
        Ok(Response::new(StatusCode::NotFound))
    }
}

async fn order_shoes(mut req: Request<AyTestState>) -> tide::Result {
    let Animal { name, legs } = req.body_json().await?;
    Ok(format!("Hello, {}! I've put in an order for {} shoes", name, legs).into())
}
