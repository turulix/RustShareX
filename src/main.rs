use std::io::Read;
use std::time::SystemTime;

use mongodb::{Client, options::ClientOptions};
use mongodb::bson::doc;
use mongodb::options::UpdateModifications::Document;
use rand::{distributions::Alphanumeric, Rng};
use serde::export::fmt::Binary;
use serde::Serialize;
use warp::{Filter, Buf};
use warp::http::HeaderValue;
use warp::hyper::body::Bytes;

mod share_x_objects;

#[tokio::main]
async fn main() {
    let client_options = ClientOptions::parse(env!("mongodb")).await;
    let client = Client::with_options(client_options.unwrap()).unwrap();

    let client_filter = warp::any().map(move || client.clone());

    let get_item = warp::get()
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and_then(load_image);

    let delete_item = warp::get()
        .and(warp::path::param::<String>())
        .and(warp::path("d"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and_then(delete_image);

    let post_item = warp::post()
        .and(warp::path("u"))
        .and(warp::path::end())
        .and(warp::body::bytes())
        .and(warp::header::value("Authorization"))
        .and(warp::header::value("Filename"))
        .and(warp::header::optional("Content-Type"))
        .and(client_filter)
        .and_then(post_image);


    let routes = get_item.or(delete_item).or(post_item);

    warp::serve(routes)
        .run(([127, 0, 0, 1], 8089))
        .await;
}


async fn load_image(image_id: String) -> Result<impl warp::Reply, warp::Rejection> {
    println!("GET {}", image_id);
    Ok("")
}

async fn delete_image(image_id: String, delete_key: String) -> Result<impl warp::Reply, warp::Rejection> {
    println!("DELETE {} {}", image_id, delete_key);
    Ok("")
}

async fn post_image(bytes: Bytes, auth: HeaderValue, filename: HeaderValue, content_type_header: Option<HeaderValue>, client: Client) -> Result<impl warp::Reply, warp::Rejection> {
    if auth != env!("password") {
        return Ok(
            warp::reply::with_status(warp::reply::json(&share_x_objects::Error::new("unauthorized".parse()?)),
                                     warp::http::status::StatusCode::UNAUTHORIZED)
        );
    }
    let max_chunk_size = 1048576; //15728640
    let chunks = bytes.chunks(max_chunk_size);

    let mut parent_id: String;
    loop {
        parent_id = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(5)
            .map(char::from)
            .collect();
        if client.database(env!("mongoDatabase"))
            .collection("mongoHeaderColl")
            .count_documents(doc! {"_id": parent_id.clone()}, None).await.unwrap() == 0 {
            break;
        }
    }

    let delete_key: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();

    let mut current_file_extension = filename.to_str().unwrap().rsplit(".");

    let mut file_extension: String = "".to_string();
    if current_file_extension.clone().count() != 1 {
        file_extension = ".".to_string() + current_file_extension.next().unwrap_or("");
    }

    let mut content_type;
    match content_type_header {
        Some(T) => {content_type = T.to_str().unwrap().parse()?},
        None => {content_type = "".parse()?}
    }

    let header_col = share_x_objects::HeaderDoc::new(parent_id.parse()?,
                                                     delete_key.parse()?,
                                                     content_type,
                                                     file_extension,
                                                     bytes.len() as u32,
                                                     SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("Time went backwards").as_millis() as u64,
                                                     chunks.clone().count() as u32);

    let mut chunk_collection: Vec<share_x_objects::ChunkDoc> = vec![];
    let mut counter = 0;

    for chunk in chunks {
        let chunk_col = share_x_objects::ChunkDoc::new(parent_id.clone(),
                                                       counter,
                                                       Vec::from(chunk));
        counter += 1;
        chunk_collection.push(chunk_col);
    }

    insert_header(&header_col, &client).await;
    insert_all_chunks(&chunk_collection, &client).await;

    return Ok(warp::reply::with_status(warp::reply::json(&header_col), warp::http::StatusCode::OK));
}

async fn insert_header(header: &share_x_objects::HeaderDoc, client: &Client) {
    let db = client.database(env!("mongoDatabase"));
    let header_col = db.collection(env!("mongoHeaderColl"));

    header_col.insert_one(doc! {
        "_id": header.id.clone(),
        "delete_key": header.delete_key.clone(),
        "content_type": header.content_type.clone(),
        "content_length": header.content_length.clone(),
        "file_extension": header.file_extension.clone(),
        "uploaded_at": header.uploaded_at.clone(),
        "total_chunks": header.total_chunks.clone()
     }, None).await;
}

async fn insert_all_chunks(chunks: &Vec<share_x_objects::ChunkDoc>, client: &Client) {
    let db = client.database(env!("mongoDatabase"));
    let chunk_col = db.collection(env!("mongoChunkColl"));

    let mut document_list = vec![];
    for x in chunks {
        document_list.push(mongodb::bson::to_document(x).unwrap());
    }
    chunk_col.insert_many(document_list, None).await;
}