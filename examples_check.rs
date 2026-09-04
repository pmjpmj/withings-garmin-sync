fn main() {
    let epoch: i64 = 1767342600; // 2026-01-02T08:30:00Z
    let dt = chrono::DateTime::from_timestamp(epoch, 0).unwrap();
    let utc = dt.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
    let local = dt.with_timezone(&chrono::Local).format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
    println!("gmt_ms   = {utc}");
    println!("local_ms = {local}");
}
