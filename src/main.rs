extern crate markup5ever_rcdom as rcdom;

use std::io::{self, BufWriter, Cursor, Read, Write};
use std::{env, sync::Arc};
use clap::{Parser, Subcommand};
use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, serialize, ParseOpts};
use once_cell::sync::Lazy;
use rcdom::{RcDom, SerializableHandle};
use regex::Regex;
use reqwest::{header, ClientBuilder, Response};
use reqwest::{cookie::Jar, Client, Url};
use skyscraper::html;
use skyscraper::xpath::{self, Xpath, XpathItemTree};

// Convenient Type

type ProcedureResult = std::result::Result<(), Box<dyn std::error::Error>>;

// Environment Variables

static WEB2PROJECT_HOST: Lazy<String> = Lazy::new(|| env::var("WEB2PROJECT_HOST").expect(r#"Environment variable "WEB2PROJECT_HOST" not set!"#));
static WEB2PROJECT_COOKIE: Lazy<String> = Lazy::new(|| env::var("WEB2PROJECT_COOKIE").expect(r#"Environment variable "WEB2PROJECT_COOKIE" not set! Authenticate with "taskblaster auth <USERNAME> <PASSWORD>" and assign the result to "WEB2PROJECT_COOKIE""#));

// Xpath

static LIST_TASKS_TR_XPATH: Lazy<Xpath> = Lazy::new(|| xpath::parse(r#"/html/body/table/tbody/tr/td/table[4]/tbody/tr/td/table/tbody/tr[3]/td/form[2]/table/tbody/tr"#).unwrap());
static LIST_TASKS_TASK_NAME_TD_XPATH: Lazy<Xpath> = Lazy::new(|| xpath::parse(r#"/td[7]/span/a"#).unwrap());

static SHOW_TASK_TASK_NAME_STRONG_XPATH: Lazy<Xpath> = Lazy::new(|| xpath::parse(r#"/html/body/table/tbody/tr/td/table[4]/tbody/tr/td[1]/table/tbody/tr[3]/td[2]/strong"#).unwrap());
static SHOW_TASK_TASK_DESCRIPTION_TD_XPATH: Lazy<Xpath> = Lazy::new(|| xpath::parse(r#"/html/body/table/tbody/tr/td/table[4]/tbody/tr/td[2]/table/tbody/tr[8]/td"#).unwrap());

// Regex

static LAST_TASKS_TASK_ID_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"project_(\d+)_level-0-task_(\d+)_").unwrap());

// CLI

#[derive(Debug, Parser)]
#[command(name = "taskblaster")]
#[command(about = "Blow away your tasks from the terminal!", long_about = None)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
	#[command(subcommand)]
	Task (TaskCommands),
	#[command(arg_required_else_help = true)]
	Auth {
		username: String,
		password: String
	}
}

#[derive(Debug, Subcommand)]
enum TaskCommands {
	#[command()]
	List,
	#[command(arg_required_else_help = true)]
	Show {
		task_id: u32
	}
}

// Utility ah jeez

async fn get_xpath_document(response: Response) -> Result<XpathItemTree, Box<dyn std::error::Error>> {
	let bytes = response.bytes().await?;
	let dom = parse_document(RcDom::default(), ParseOpts::default()).from_utf8().read_from(&mut Cursor::new(bytes))?;
	let document: SerializableHandle = dom.document.clone().into();
	let mut buf = BufWriter::new(Vec::new());
	serialize(&mut buf, &document, Default::default())?;
	let buf_bytes = buf.into_inner()?;
	let text = String::from_utf8(buf_bytes)?;
	let html_document = html::parse(&text)?;
	return Ok(XpathItemTree::from(&html_document));
}

// Methods

async fn authenticate(client: &Client, username: &str, password: &str) -> ProcedureResult {
	let form = reqwest::multipart::Form::new()
		.text("login", "login")
		.text("username", username.to_owned())
		.text("password", password.to_owned());
	let response = client.execute(client.post(format!("https://{}/index.php", *WEB2PROJECT_HOST)).multipart(form).build()?).await?;
	println!("{}", response.cookies().find(|cookie| cookie.name() == "web2project").unwrap().value());
	return Ok(());
}

async fn list_tasks(client: &Client) -> ProcedureResult {
	let response = client.execute(client.get(format!("https://{}/index.php?m=tasks&a=todo", *WEB2PROJECT_HOST)).build()?).await?;
	let xpath_document = get_xpath_document(response).await?;
	let tr_items = LIST_TASKS_TR_XPATH.apply(&xpath_document)?;
	for tr_item in tr_items {
		let tr_node = tr_item.as_node()?.as_tree_node()?;
		let tr_element = tr_node.data.extract_as_element_node();
		if let Some(id) = tr_element.get_attribute("id") {
			let captures = LAST_TASKS_TASK_ID_REGEX.captures(id).unwrap();
			let task_id = captures.get(2).unwrap().as_str().parse::<u32>()?;
			let td_items = LIST_TASKS_TASK_NAME_TD_XPATH.apply_to_item(&xpath_document, tr_item)?;
			let td_item = td_items.iter().next().unwrap();
			let td_node = td_item.as_node()?.as_tree_node()?;
			let task_name = td_node.text(&xpath_document).unwrap();
			println!("{} {}", task_id, task_name);
		}
	}
	return Ok(());
}

async fn show_task(client: &Client, task_id: u32) -> ProcedureResult {
	let response = client.execute(client.get(format!("https://{}/index.php?m=tasks&a=view&task_id={}", *WEB2PROJECT_HOST, task_id)).build()?).await?;
	let xpath_document = get_xpath_document(response).await?;
	let task_name = SHOW_TASK_TASK_NAME_STRONG_XPATH.apply(&xpath_document)?.iter().next().unwrap().as_node()?.as_tree_node()?.text(&xpath_document).unwrap();
	let task_description = SHOW_TASK_TASK_DESCRIPTION_TD_XPATH.apply(&xpath_document)?.iter().next().unwrap().as_node()?.as_tree_node()?.text(&xpath_document).unwrap();
	println!("{} {}", task_id, task_name);
	println!();
	println!("{}", task_description);
	return Ok(());
}

fn get_client_builder() -> ClientBuilder {
	let mut headers = header::HeaderMap::new();
	headers.insert("Accept", header::HeaderValue::from_static("text/html"));

	return Client::builder()
		.user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36") // I'm a browser :y
		.default_headers(headers);
}

fn get_client() -> Result<Client, Box<dyn std::error::Error>> {
	return Ok(get_client_builder().build()?);
}

fn get_authed_client() -> Result<Client, Box<dyn std::error::Error>> {
	let jar = Jar::default();
	let url = format!("https://{}", *WEB2PROJECT_HOST).parse::<Url>().unwrap();
	jar.add_cookie_str(format!("web2project={}", *WEB2PROJECT_COOKIE).as_str(), &url);
	return Ok(get_client_builder()
		.cookie_store(true)
		.cookie_provider(Arc::new(jar))
		.build()?);
}

#[tokio::main]
async fn main() -> ProcedureResult {
	let args = Cli::parse();

	match args.command {
		Commands::Task (task_command) => {
			match task_command {
				TaskCommands::List => {
					let client = get_authed_client()?;
					list_tasks(&client).await?;
				},
				TaskCommands::Show { task_id } => {
					let client = get_authed_client()?;
					show_task(&client, task_id).await?;
				}
			}
		},
		Commands::Auth { username, password } => {
			let client = get_client()?;
			authenticate(&client, &username, &password).await?;
		}
	}

	Ok(())
}
