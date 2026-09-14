

pub struct Extensions {
    audio: Vec<String>,
    documents: Vec<String>,
    image: Vec<String>,
    video: Vec<String>,
    // For custom extensions
    custom: Vec<String>
}

impl Default for Extensions {
    fn default() -> Self {
        Self {
            audio: vec![
                "mp3".to_string(),
                "flac".to_string(),
                "wav".to_string(),
                "aac".to_string(),
                "ogg".to_string(),
                "m4a".to_string(),
            ],
            documents: vec![
                "pdf".to_string(),
                "txt".to_string(),
                "md".to_string(),
                "doc".to_string(),
                "docx".to_string(),
                "odt".to_string(),
            ],
            image: vec![
                "jpg".to_string(),
                "jpeg".to_string(),
                "png".to_string(),
                "webp".to_string(),
                "gif".to_string(),
                "bmp".to_string(),
            ],
            video: vec![
                "mp4".to_string(),
                "mkv".to_string(),
                "mov".to_string(),
                "avi".to_string(),
                "webm".to_string(),
            ],
            custom: vec![],
        }
    }
}
