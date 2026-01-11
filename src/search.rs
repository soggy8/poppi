use std::process::Command;

pub struct WebSearch;

impl WebSearch {
    pub fn search_youtube(query: &str) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("https://www.youtube.com/results?search_query={}", 
                         urlencoding::encode(query));
        Self::open_url(&url)
    }

    pub fn search_chatgpt(_query: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Note: ChatGPT doesn't have a direct search URL, but we can open the main page
        // Users would need to paste their query. Alternatively, we could use the API.
        let url = "https://chat.openai.com/";
        Self::open_url(url)
    }

    pub fn search_google(query: &str) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("https://www.google.com/search?q={}", 
                         urlencoding::encode(query));
        Self::open_url(&url)
    }

    fn open_url(url: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Try xdg-open first (works on most Linux systems)
        if Command::new("xdg-open").arg(url).spawn().is_ok() {
            return Ok(());
        }

        // Fallback to other methods
        let browsers = vec!["firefox", "google-chrome", "chromium", "brave-browser"];
        for browser in browsers {
            if Command::new(browser).arg(url).spawn().is_ok() {
                return Ok(());
            }
        }

        Err("Could not open browser".into())
    }

    pub fn is_search_query(query: &str, prefix: &str) -> bool {
        query.trim().to_lowercase().starts_with(&prefix.to_lowercase())
    }

    pub fn extract_query(query: &str, prefix: &str) -> String {
        query.trim()[prefix.len()..].trim().to_string()
    }
}

