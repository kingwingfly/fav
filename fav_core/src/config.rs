trait Config {
    fn default_headers(&self) -> http::HeaderMap;
}
