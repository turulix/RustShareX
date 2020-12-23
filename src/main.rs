use std::borrow::Borrow;
use std::cmp::Ordering;
use std::io::Write;
use std::time::SystemTime;

use mongodb::{Client, options::ClientOptions};
use mongodb::bson::doc;
use rand::{distributions::Alphanumeric, Rng};
use tokio::stream::StreamExt;
use warp::{Buf, Filter};
use warp::http::{HeaderValue, Response};
use warp::hyper::Body;
use warp::hyper::body::Bytes;
use warp::reply::{Json, WithStatus};

mod share_x_objects;

#[tokio::main]
async fn main() {
    let client_options = ClientOptions::parse(env!("mongodb")).await;
    let client = Client::with_options(client_options.unwrap()).unwrap();

    let client_filter = warp::any().map(move || client.clone());

    // Setup load_image route /<id>
    let get_item = warp::get()
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(client_filter.clone())
        .and_then(load_image);

    // Setup load_image route /<id>/d/<del_key>
    let delete_item = warp::get()
        .and(warp::path::param::<String>())
        .and(warp::path("d"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(client_filter.clone())
        .and_then(delete_image);

    // Setup load_image route /u
    let post_item = warp::post()
        .and(warp::path("u"))
        .and(warp::path::end())
        .and(warp::body::bytes())
        .and(warp::header::value("Authorization"))
        .and(warp::header::value("Filename"))
        .and(warp::header::optional("Content-Type"))
        .and(client_filter.clone())
        .and_then(post_image);


    let routes = get_item.or(delete_item).or(post_item);

    // Start the server and bind it to 127.0.0.1
    //TODO: Do not hardcode this
    warp::serve(routes)
        .run(([127, 0, 0, 1], 8089))
        .await;
}


async fn load_image(image_id: String, _client: Client) -> Result<impl warp::Reply, warp::Rejection> {

    // Filter out file extension
    let real_id = &image_id.split(".").next().unwrap().parse().unwrap();

    // Get the HeaderDocument return error if none is found.
    let header_doc = match get_header(real_id, &_client).await {
        Some(t) => t,
        None => return Ok(warp::Reply::into_response(reply_error("not_found", warp::http::StatusCode::NOT_FOUND)))
    };

    // Get all the ChunkDocuments return error if none is found.
    let mut chunks = match get_all_chunks(&header_doc.id, &_client).await {
        Some(t) => t,
        None => return Ok(warp::Reply::into_response(reply_error("no_chunks_found", warp::http::StatusCode::INTERNAL_SERVER_ERROR)))
    };

    //Sort all the chunks to make sure they are in the correct order.
    chunks.sort_by_key(|a| a.index);

    // Combine all the chunks into one array
    let mut bytes = Vec::new();
    for x in chunks {
        bytes.append(&mut x.data.clone()) //TODO: Maybe optimise this?
    }

    // Send the response as raw bytes
    let reply = warp::reply::with_header(bytes, "Content-Type", &header_doc.content_type);
    println!("Served file \"{}\" of type \"{}\" with total size \"{} ({})\"", &header_doc.id, &header_doc.content_type, &header_doc.content_length, &header_doc.total_chunks);
    return Ok(warp::Reply::into_response(warp::reply::with_status(reply, warp::http::StatusCode::OK)));
}

async fn delete_image(image_id: String, delete_key: String, _client: Client) -> Result<impl warp::Reply, warp::Rejection> {
    // Get the HeaderDocument return error if none is found.
    let header = match get_header(&image_id, &_client).await {
        Some(t) => t,
        None => return Ok(reply_error("not_found", warp::http::StatusCode::NOT_FOUND))
    };

    // Check if the delete_key is valid for this file
    if header.delete_key != delete_key {
        return Ok(reply_error("invalid_delete_key", warp::http::StatusCode::FORBIDDEN));
    }

    // Delete the file
    delete_file(&image_id, &_client).await;
    println!("Deleted file \"{}\" of type \"{}\" with total size \"{} ({})\"", &header.id, &header.content_type, &header.content_length, &header.total_chunks);
    return Ok(reply_error("", warp::http::StatusCode::OK));
}

async fn post_image(bytes: Bytes, auth: HeaderValue, filename: HeaderValue, content_type_header: Option<HeaderValue>, client: Client) -> Result<impl warp::Reply, warp::Rejection> {
    // Check if the upload request is authorized.
    if auth != env!("password") {
        return Ok(reply_error("unauthorized", warp::http::StatusCode::UNAUTHORIZED));
    }

    // Split the uploaded file into chunks of 15 MB because of the MongoDB document limit.
    let max_chunk_size = 15728640; // About 15 MB (leaving 1MB to spare)
    let chunks = bytes.chunks(max_chunk_size);

    let mut parent_id: String;
    loop {
        //Generate random string
        parent_id = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(5)
            .map(char::from)
            .collect();
        //Ensure that string is unique
        if client.database(env!("mongoDatabase"))
            .collection("mongoHeaderColl")
            .count_documents(doc! {"_id": parent_id.clone()}, None).await.unwrap() == 0 {
            break;
        }
    }

    // Just generate a random deletion key.
    let delete_key: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();

    // Gets the file extension
    let mut current_file_extension = filename.to_str().unwrap().rsplit(".");

    let mut file_extension: String = String::new();
    if current_file_extension.clone().count() != 1 {
        file_extension = ".".to_string() + current_file_extension.next().unwrap_or("");
    }

    // Gets the Content-Type send by ShareX
    let content_type;
    match content_type_header {
        Some(t) => { content_type = t.to_str().unwrap().parse()? }
        None => { content_type = String::new() }
    }

    // Create the header document
    let header_col = share_x_objects::HeaderDoc::new(parent_id.clone(),
                                                     delete_key.clone(),
                                                     content_type,
                                                     file_extension,
                                                     bytes.len() as u32,
                                                     SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                                                     chunks.clone().count() as u32);

    //Create the array of chunk documents
    let mut chunk_collection: Vec<share_x_objects::ChunkDoc> = Vec::new();
    let mut index = 0;
    for chunk in chunks {
        index += 1;
        chunk_collection.push(share_x_objects::ChunkDoc::new(parent_id.clone(),
                                                             index,
                                                             Vec::from(chunk)));
    }

    // Add to database
    insert_header(&header_col, &client).await;
    insert_all_chunks(&chunk_collection, &client).await;

    println!("Uploaded new file \"{}\" of type \"{}\" with total size \"{} ({})\"", &header_col.id, &header_col.content_type, &header_col.content_length, &header_col.total_chunks);
    return Ok(warp::reply::with_status(warp::reply::json(&header_col), warp::http::StatusCode::OK));
}


fn reply_error(error: &str, status: warp::http::status::StatusCode) -> WithStatus<Json> {
    return warp::reply::with_status(warp::reply::json(&share_x_objects::Error::new(error.to_string())), status);
}

async fn delete_file(id: &String, client: &Client) -> Option<(i64, i64)> {
    let db = client.database(env!("mongoDatabase"));
    let header_col = db.collection(env!("mongoHeaderColl"));
    let chunk_col = db.collection(env!("mongoChunkColl"));

    // Delete the header document from the database
    let header_result = match header_col.delete_one(doc! {"_id": id}, None).await {
        Ok(t) => t,
        Err(e) => {
            println!("Something went wrong deleting header for {}", id);
            return None;
        }
    };

    // Delete all the chunk documents from the database
    let chunk_result = match chunk_col.delete_many(doc! {"parent_id": id}, None).await {
        Ok(t) => t,
        Err(e) => {
            println!("Something went wrong deleting chunks for {}", id);
            return None;
        }
    };

    return Some((header_result.deleted_count, chunk_result.deleted_count));
}

async fn get_header(id: &String, client: &Client) -> Option<share_x_objects::HeaderDoc> {
    let db = client.database(env!("mongoDatabase"));
    let header_col = db.collection(env!("mongoHeaderColl"));

    // Gets the header document from the database
    let results = header_col.find_one(doc! {"_id": id}, None).await.unwrap(); //TODO: Don't unwrap this
    return match results {
        Some(t) => Some(mongodb::bson::from_document::<share_x_objects::HeaderDoc>(t).unwrap()), //TODO: Don't unwrap this
        None => None
    };
}

async fn get_all_chunks(id: &String, client: &Client) -> Option<Vec<share_x_objects::ChunkDoc>> {
    let db = client.database(env!("mongoDatabase"));
    let chunk_col = db.collection(env!("mongoChunkColl"));

    let mut cursor = chunk_col.find(doc! {"parent_id": id}, None).await.unwrap(); //TODO: Don't unwrap this
    let mut documents = Vec::new();

    // Collect all the chunk documents and add them to a vector
    while let Some(result) = cursor.next().await {
        match result {
            Ok(t) => {
                documents.push(mongodb::bson::from_document::<share_x_objects::ChunkDoc>(t).unwrap()); // TODO: Should be fine but don't
            }
            Err(e) => return None
        }
    }

    return Some(documents);
}

async fn insert_header(header: &share_x_objects::HeaderDoc, client: &Client) {
    let db = client.database(env!("mongoDatabase"));
    let header_col = db.collection(env!("mongoHeaderColl"));

    // Create a new header document and insert it to the Database
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

    // Convert ChunkDocuments to mongodb documents and insert them to the database
    let mut document_list = vec![];
    for x in chunks {
        document_list.push(mongodb::bson::to_document(x).unwrap()); //TODO: Don't unwrap this
    }
    chunk_col.insert_many(document_list, None).await;
}