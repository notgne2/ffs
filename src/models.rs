use super::schema::{joins, points, tags};

#[derive(Identifiable, Queryable, Associations, Debug, Clone)]
pub struct Point {
    pub id: i32,
    pub name: String,
    pub path: Option<String>,
    pub hash: String,
    pub dir: bool,
}

#[derive(Insertable, Debug)]
#[table_name = "points"]
pub struct NewPoint {
    pub id: i32,
    pub name: String,
    pub path: Option<String>,
    pub hash: String,
    pub dir: bool,
}

#[derive(Identifiable, Queryable, Associations, Debug, Clone)]
pub struct Tag {
    pub id: i32,
    pub name: String,
    pub value: Option<String>,
    pub sort_value: Option<i64>,
}

#[derive(Insertable, Debug)]
#[table_name = "tags"]
pub struct NewTag {
    pub id: i32,
    pub name: String,
    pub value: Option<String>,
    pub sort_value: Option<i64>,
}

#[derive(Identifiable, Queryable, Associations, Debug, Clone)]
#[belongs_to(Point)]
#[belongs_to(Tag)]
pub struct Join {
    pub id: i32,
    pub tag_id: i32,
    pub point_id: i32,
}

#[derive(Insertable, Debug)]
#[table_name = "joins"]
pub struct NewJoin {
    pub id: i32,
    pub tag_id: i32,
    pub point_id: i32,
}
