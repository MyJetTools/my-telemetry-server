use ring::digest::{Context, Digest, SHA256};
use rust_extensions::base64::IntoBase64;
use std::io::Read;

pub fn calc(data: &str) -> String {
    let result = sha256_digest(data.as_bytes());
    let result = result.as_ref().into_base64();
    result
}

fn sha256_digest<R: Read>(mut reader: R) -> Digest {
    let mut context = Context::new(&SHA256);
    let mut buffer = [0; 1024];

    loop {
        let count = reader.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        context.update(&buffer[..count]);
    }

    context.finish()
}
