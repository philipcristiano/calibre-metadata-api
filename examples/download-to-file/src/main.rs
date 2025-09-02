use clap::Parser;
use serde::{Deserialize, Serialize};
use tera::Tera;
use std::fs::File;
use std::io::prelude::*;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long, default_value = "http://127.0.0.1:3002")]
    api_addr: String,
    #[arg(short, long, default_value = "./output")]
    output_path: String,
    #[arg(short, long, default_value = "./templates")]
    template_path: String,

}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct V1APIResponse {
    data: Vec<CDBStruct>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum CDBStruct {
    Author(Author),
    Book(Book),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Author {
    id: i64,
    name: String,
    sort: Option<String>,
    link: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Book {
    id: i64,
    title: String,
    // With calibre-web, the isbn is always empty in this table
    //isbn: Option<String>,
    //pubdate: Option<chrono::NaiveDateTime>,
    author_name: String,
    author_id: i64,
}


#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();

    let templates_glob = format!("{}/*", args.template_path);
    let templates = Tera::new(templates_glob.as_str())?;
    let client = reqwest::Client::new();
    let authors_path = format!("{}/v1/authors", args.api_addr);


    let resp = client.get(authors_path).send().await?;
    let api_resp = resp.json::<V1APIResponse>().await?;
    for item in api_resp.data {
        match item {
            CDBStruct::Author(author) => {
                let context = &tera::Context::from_serialize(&author)?;
                println!("Author {author:?}");
                let path = templates.render("author_filename", &context)?;
                let path = format!("{}/{}", args.output_path, path.trim());
                let data = templates.render("author", &context)?;
                println!("Will write file: {path}");
                let maybe_file = File::create(&path);
                if let Ok(mut file) = maybe_file {
                    file.write_all(data.as_bytes())?;
                } else {
                    eprintln!("Error opening file: {path}: {:?}", maybe_file)
                }
            }
            _ => println!("Unprocessed item {item:?}"),
        }
    }

    let books_path = format!("{}/v1/books", args.api_addr);
    let resp = client.get(books_path).send().await?;
    let api_resp = resp.json::<V1APIResponse>().await?;

    for item in api_resp.data {
        match item {
            CDBStruct::Book(book) => {
                let context = &tera::Context::from_serialize(&book)?;
                let path = templates.render("book_filename", &context)?;
                let path = format!("{}/{}", args.output_path, path.trim());
                let data = templates.render("book", &context)?;
                println!("Will write file: {path}");
                let maybe_file = File::create(&path);
                if let Ok(mut file) = maybe_file {
                    file.write_all(data.as_bytes())?;
                } else {
                    eprintln!("Error opening file: {path}: {:?}", maybe_file)
                }


            }


            _ => println!("Unprocessed item {item:?}"),

        }
    }


    Ok(())
}
