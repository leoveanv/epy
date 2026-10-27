pub struct SvgBuilder {
    pub width: u32,
    pub height: u32,
    pub body: String,
}

impl SvgBuilder {
    pub fn new() -> Self {
        Self {
            width: 8,
            height: 6,
            body: "<rect width='100%' height='100%' fill='red'/>".into(),
        }
    }
    pub fn bytes(&self) -> Vec<u8> {
        format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='{}' height='{}'>{}</svg>",
            self.width, self.height, self.body
        )
        .into_bytes()
    }
}
