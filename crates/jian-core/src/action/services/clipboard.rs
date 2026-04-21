use async_trait::async_trait;

#[async_trait(?Send)]
pub trait ClipboardService {
    async fn read_text(&self) -> Option<String>;
    async fn write_text(&self, text: &str);
}
