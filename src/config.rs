

pub struct Extensions {
    audio: Vec<String>,
    documents: Vec<String>,
    image: Vec<String>,
    video: Vec<String>,
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
        }
    }
}

impl Extensions {
    pub fn get_extensios(&self, extension_type: &str) -> Vec<String> {
        match extension_type {
            "audio" => self.audio.clone(),
            "documents" => self.documents.clone(),
            "image" => self.image.clone(),
            "video" => self.video.clone(),
            _ => vec![],
        }
    }

    /// A qué grupo pertenece una extensión, para elegir el visor adecuado.
    /// Si no está en ninguno de los grupos predefinidos, se considera custom.
    pub fn category_for(&self, extension: &str) -> FileCategory {
        if self.image.iter().any(|e| e.eq_ignore_ascii_case(extension)) {
            FileCategory::Image
        } else if self.video.iter().any(|e| e.eq_ignore_ascii_case(extension)) {
            FileCategory::Video
        } else if self.audio.iter().any(|e| e.eq_ignore_ascii_case(extension)) {
            FileCategory::Audio
        } else if self.documents.iter().any(|e| e.eq_ignore_ascii_case(extension)) {
            FileCategory::Documents
        } else {
            FileCategory::Custom
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Image,
    Video,
    Audio,
    Documents,
    Custom,
}
