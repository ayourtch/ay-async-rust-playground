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

use async_ssh2::Session;
use smol::Async;
use std::net::TcpStream;

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
    app.at("/request").get(request_url);
    app.at("/tcptest").get(tcptest);
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
    app.listen("0.0.0.0:8080").await?;
    Ok(())
}

async fn upload_file(mut req: Request<AyTestState>) -> tide::Result<serde_json::Value> {
    let path = req.param("file")?.to_string();
    let fs_path = req.state().path().join(&path);

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

    let mut metadata: HashMap<String, String> = HashMap::new();
    let fpath = fs_path.canonicalize()?.to_str().unwrap().to_string();
    if let Some(file) = std::fs::File::open(&fpath).ok() {
        tide::log::info!("reading exif", { fpath: fpath });
        let mut bufreader = std::io::BufReader::new(&file);
        let exifreader = exif::Reader::new();
        let exif = exifreader.read_from_container(&mut bufreader)?;
        for f in exif.fields() {
            metadata.insert(
                format!("{} {}", f.tag, f.ifd_num),
                format!("{}", f.display_value().with_unit(&exif)),
            );
        }
    }
    eprintln!("Metadata: {:?}", &metadata);

    Ok(json!({ "bytes": bytes_written, "meta": metadata }))
}

#[derive(Deserialize)]
struct RequestQuery {
    url: String,
}

async fn request_url(mut req: Request<AyTestState>) -> tide::Result {
    let RequestQuery { url } = req.query().unwrap();
    let mut res: surf::Response = surf::get(url).await?;
    let data: String = res.body_string().await?;

    Ok(data.into())
}

#[derive(Deserialize)]
struct TcpTestQuery {
    target: String,
    user: String,
    pass: String,
    command: String,
}

async fn tcptest(mut req: Request<AyTestState>) -> tide::Result {
    use async_ssh2::Session;
    use async_std::io::ReadExt;
    use async_std::io::WriteExt;
    use async_std::net::SocketAddr;
    use smol::Async;
    use std::net::TcpStream;

    let TcpTestQuery {
        target,
        user,
        pass,
        command,
    } = req.query()?;
    let server: SocketAddr = target.parse()?;
    let mut stream = Async::<TcpStream>::connect(server).await?;

    // one example: https://users.rust-lang.org/t/strange-behaviour-of-async-sftp/62671

    let mut session = Session::new()?;
    session.set_tcp_stream(stream);
    session.handshake().await?;

    session.userauth_password(&user, &pass).await?;
    let mut channel = session.channel_session().await?;
    channel.exec(&command).await?;
    let mut s = String::new();
    channel.read_to_string(&mut s).await?;

    let data = format!("result: {}", s);
    Ok(data.into())
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
