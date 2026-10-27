use ers::annotations::AnnotationBook;

pub fn book() -> AnnotationBook {
    AnnotationBook {
        id: "00000000-0000-4000-8000-000000000001".into(),
        fingerprint: "a".repeat(64),
        title: "Synthetic book".into(),
        author: "Fixture author".into(),
    }
}
