use serde::{Deserialize, Serialize };

#[derive(Serialize, Deserialize, Debug)]
pub struct HeaderDoc{
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(rename = "delete_key")]
    pub delete_key: String,
    #[serde(rename = "content_type")]
    pub content_type: String,
    #[serde(rename = "file_extension")]
    pub file_extension: String,
    #[serde(rename = "content_length")]
    pub content_length: u32,
    #[serde(rename = "uploaded_at")]
    pub uploaded_at: u64,
    #[serde(rename = "total_chunks")]
    pub total_chunks: u32,
}

impl HeaderDoc {
    pub fn new(id: String, delete_key: String, content_type: String, file_extension: String, content_length: u32, uploaded_at: u64, total_chunks: u32) -> Self {
        HeaderDoc { id, delete_key, content_type, file_extension, content_length, uploaded_at, total_chunks }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChunkDoc{
    #[serde(rename = "parent_id")]
    pub parent_id: String,
    pub index: i32,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>
}

impl ChunkDoc {
    pub fn new(parent_id: String, index: i32, data: Vec<u8>) -> Self {
        ChunkDoc { parent_id, index, data }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Error {
    error: String
}

impl Error {
    pub fn new(error: String) -> Self {
        Error { error }
    }
}