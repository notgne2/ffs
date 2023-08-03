use super::{schema, Join, Point, SqliteConnection, Tag, QUERY_RE};
use blake2::{Blake2b512, Digest};
use diesel::prelude::*;
use std::path::Path;
use std::{fs, io};

pub type TagContent = Option<(String, Option<i64>)>;
pub type TagEntry = (String, TagContent);
pub type TagEntries = Vec<TagEntry>;

pub fn hash_path<T: AsRef<Path>>(path: T) -> (String, bool) {
    let md = fs::metadata(&path).unwrap();

    let mut hasher = Blake2b512::new();

    let dir = md.is_dir();
    if dir {
        for entry in walkdir::WalkDir::new(&path) {
            let entry = entry.unwrap();

            if !entry.file_type().is_file() {
                continue;
            }

            let mut file = fs::File::open(entry.path()).expect("walkdir dogged the boys");
            io::copy(&mut file, &mut hasher).expect("error reading file");
        }
    } else {
        let mut file = fs::File::open(&path).expect("give me a valid path");
        io::copy(&mut file, &mut hasher).expect("error reading file");
    };

    let hash = hasher.finalize();
    (hex::encode(hash), dir)
}

pub fn get_tags_for_point(connection: &SqliteConnection, point: &Point) -> Vec<Tag> {
    use schema::{joins, tags};

    let tag_ids = Join::belonging_to(point).select(joins::tag_id);

    tags::table
        .filter(tags::id.eq_any(tag_ids))
        .load::<Tag>(connection)
        .expect("could not load tags")
}

pub fn get_tags_for_points(connection: &SqliteConnection, points: &Vec<Point>) -> Vec<Tag> {
    use schema::{joins, tags};

    let tag_ids = Join::belonging_to(points).select(joins::tag_id);

    tags::table
        .filter(tags::id.eq_any(tag_ids))
        .load::<Tag>(connection)
        .expect("could not load tags")
}

pub fn get_tags_by_parts(connection: &SqliteConnection, path_parts: &[&str]) -> Vec<Vec<Tag>> {
    use schema::tags;

    let mut part_tags: Vec<Vec<Tag>> = Vec::new();

    for path_part in path_parts {
        let mut these_tags = Vec::new();

        for tag_query in path_part.split(" or ") {
            let split_query = QUERY_RE.captures(tag_query);

            let found_tags = match split_query {
                Some(caps) => match (
                    caps.get(1).map(|x| x.as_str()),
                    caps.get(2).map(|x| x.as_str()),
                    caps.get(3).map(|x| x.as_str()),
                    caps.get(3).map(|x| x.as_str().parse::<i64>()),
                ) {
                    (Some(name), Some(">"), _, Some(Ok(sort_value))) => tags::dsl::tags
                        .filter(tags::dsl::name.eq(name))
                        .filter(tags::dsl::sort_value.gt(sort_value))
                        .load::<Tag>(connection)
                        .expect("Error loading tags"),
                    (Some(name), Some("<"), _, Some(Ok(sort_value))) => tags::dsl::tags
                        .filter(tags::dsl::name.eq(name))
                        .filter(tags::dsl::sort_value.lt(sort_value))
                        .load::<Tag>(connection)
                        .expect("Error loading tags"),
                    (Some(name), Some("!="), _, Some(Ok(sort_value))) => tags::dsl::tags
                        .filter(tags::dsl::name.eq(name))
                        .filter(tags::dsl::sort_value.ne(sort_value))
                        .load::<Tag>(connection)
                        .expect("Error loading tags"),
                    (Some(name), Some("="), _, Some(Ok(sort_value))) => tags::dsl::tags
                        .filter(tags::dsl::name.eq(name))
                        .filter(tags::dsl::sort_value.eq(sort_value))
                        .load::<Tag>(connection)
                        .expect("Error loading tags"),
                    (Some(name), Some("!="), Some(value), _) => tags::dsl::tags
                        .filter(tags::dsl::name.eq(name))
                        .filter(tags::dsl::value.ne(value))
                        .load::<Tag>(connection)
                        .expect("Error loading tags"),
                    (Some(name), Some("="), Some(value), _) => tags::dsl::tags
                        .filter(tags::dsl::name.eq(name))
                        .filter(tags::dsl::value.eq(value))
                        .load::<Tag>(connection)
                        .expect("Error loading tags"),
                    _ => vec![],
                },
                None => tags::dsl::tags
                    .filter(tags::dsl::name.eq(tag_query.to_string()))
                    .load::<Tag>(connection)
                    .expect("Error loading tags"),
            };

            these_tags.extend(found_tags);
        }

        part_tags.push(these_tags);
    }

    part_tags
}

pub fn get_points_by_parts(connection: &SqliteConnection, path_parts: &[&str]) -> Vec<Point> {
    use schema::joins;
    use schema::points;

    if path_parts.is_empty() {
        let mut points: Vec<Point> = Vec::new();

        for point in points::dsl::points
            .load::<Point>(connection)
            .expect("Error loading points")
        {
            points.push(point);
        }

        points
    } else {
        let found_tags_per_part = get_tags_by_parts(connection, path_parts);

        let mut points_per_part: Vec<Vec<i32>> = Vec::new();

        let mut point_ids_as_of_now = Vec::new();

        for found_tags_for_part in &found_tags_per_part {
            let mut points_for_this_part = Vec::new();

            for found_tag in found_tags_for_part {
                let joins = joins::dsl::joins
                    .filter(joins::dsl::tag_id.eq(found_tag.id))
                    .load::<Join>(connection)
                    .expect("Error loading joins");

                for join in joins {
                    if !points_for_this_part.contains(&join.point_id) {
                        points_for_this_part.push(join.point_id);
                    }

                    if !point_ids_as_of_now.contains(&join.point_id) {
                        point_ids_as_of_now.push(join.point_id);
                    }
                }
            }

            points_per_part.push(points_for_this_part);
        }

        for points_for_this_part in points_per_part.iter() {
            point_ids_as_of_now.retain(|x| points_for_this_part.contains(x));
        }

        points::dsl::points
            .filter(points::dsl::id.eq_any(point_ids_as_of_now))
            .load::<Point>(connection)
            .expect("Error loading points")
    }
}
