// Fixture for GenericParam edge extraction. The structural pass should emit
// a GenericParam edge from the file to each generic type parameter referenced
// via `Vec<User>`, `Map<K, V>`, etc.

pub struct User {
    pub id: u64,
    pub name: String,
}

pub struct Session {
    pub token: String,
}

pub fn load_users() -> Vec<User> {
    Vec::new()
}

pub fn build_session_map() -> std::collections::HashMap<String, Session> {
    std::collections::HashMap::new()
}

pub fn chain<T, U>(first: Vec<User>, second: Vec<Session>) {
    let _ = (first, second);
}
