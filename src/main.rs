#![feature(proc_macro_hygiene)]
#![feature(let_else)]

use self::diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use lazy_static::lazy_static;
use rand::prelude::*;
use regex::Regex;
use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

extern crate pretty_env_logger;
#[macro_use]
extern crate log;
#[macro_use]
extern crate diesel;
#[macro_use]
extern crate serde_derive;

extern crate magic;

lazy_static! {
    pub static ref QUERY_RE: Regex = Regex::new(r"(\w+)\s*(<|>|=|!=)\s*(.+)").unwrap();
    pub static ref INFINITE_QUERY_RE: Regex = Regex::new(r"(\w+)\s*(<|>|!=)\s*(.+)").unwrap();
}

mod autotagger;
mod ffs;
mod models;
pub mod schema;
mod utils;

use autotagger::get_generic_tags_from_file;
use ffs::*;
pub use models::*;
use utils::*;

#[derive(Deserialize, Debug, Clone)]
pub struct FfsConfig {
    magic_file: String,
    db_url: String,
    store_dir: Option<String>,
    delegate_dirs: Vec<String>,
}

fn random_id() -> i32 {
    let mut rng = rand::thread_rng();
    // rng.gen_range(0..=i32::MAX)
    rng.gen_range(0..1_000_000)
}

fn tag_point(
    connection: &SqliteConnection,
    id: i32,
    tag_name: String,
    tag_content: Option<(String, Option<i64>)>,
) {
    use schema::{joins, tags};

    let mut existing_tags = (match tag_content {
        Some((ref tag_value, _)) => tags::dsl::tags
            .filter(tags::dsl::name.eq(&tag_name))
            .filter(tags::dsl::value.eq(tag_value))
            .limit(1)
            .load::<Tag>(connection),
        None => tags::dsl::tags
            .filter(tags::dsl::name.eq(&tag_name))
            .limit(1)
            .load::<Tag>(connection),
    })
    .expect("error searching tags");

    let tag_id = match existing_tags.pop() {
        Some(t) => t.id,
        None => {
            let tag_id = random_id();

            let (tag_value, tag_sort_value) = match tag_content {
                Some((tag_value, Some(tag_sort_value))) => (Some(tag_value), Some(tag_sort_value)),
                Some((tag_value, None)) => (Some(tag_value), None),
                None => (None, None),
            };

            diesel::insert_into(tags::table)
                .values(&NewTag {
                    id: tag_id,
                    name: tag_name,
                    value: tag_value,
                    sort_value: tag_sort_value,
                })
                .execute(connection)
                .expect("Error saving new point");

            tag_id
        }
    };

    let existing_joins = joins::dsl::joins
        .filter(joins::dsl::tag_id.eq(tag_id))
        .filter(joins::dsl::point_id.eq(&id))
        .limit(1)
        .load::<Join>(connection)
        .expect("error searching joins");

    if existing_joins.get(0).is_none() {
        diesel::insert_into(joins::table)
            .values(&NewJoin {
                id: random_id(),
                point_id: id,
                tag_id,
            })
            .execute(connection)
            .expect("Error saving new join");
    }
}

fn update_point_by_path<'a>(
    connection: &'a SqliteConnection,
    name: String,
    path_str: &str,
    magic_file: &str,
    tags: TagEntries,
) {
    use schema::points;

    let path = Path::new(path_str);

    let (hash, dir) = utils::hash_path(&path);

    let existing_points_by_path = points::dsl::points
        .filter(points::dsl::path.eq(path_str))
        .limit(1)
        .load::<Point>(connection)
        .expect("error searching points");

    let existing_points = points::dsl::points
        .filter(points::dsl::hash.eq(&hash))
        .limit(1)
        .load::<Point>(connection)
        .expect("error searching points");

    let (maybe_point, point_id) = match existing_points_by_path.get(0) {
        Some(x) => (Some(x), x.id),
        None => match existing_points.get(0) {
            Some(x) => (Some(x), x.id),
            None => {
                let tag_id = random_id();

                diesel::insert_into(points::table)
                    .values(&NewPoint {
                        id: tag_id,
                        name,
                        path: Some(path_str.to_string()),
                        hash: hash.to_string(),
                        dir,
                    })
                    .execute(connection)
                    .expect("Error saving new point");

                (None, tag_id)
            }
        },
    };

    for (tag_name, tag_content) in tags {
        tag_point(connection, point_id, tag_name, tag_content)
    }

    let point = match maybe_point {
        Some(x) => x.clone(),
        None => points::dsl::points
            .find(point_id)
            .first::<Point>(connection)
            .unwrap(),
    };

    update_point(connection, magic_file, Some(path_str), Some(&hash), &point);
}

fn update_point(
    connection: &SqliteConnection,
    magic_file: &str,
    new_path: Option<&str>,
    new_hash: Option<&str>,
    point: &Point,
) {
    use schema::points;

    let path = match (&point.path, new_path) {
        (None, Some(new_path)) => {
            diesel::update(points::dsl::points.find(point.id))
                .set(points::dsl::path.eq(new_path))
                .execute(connection)
                .expect("Error updating point");

            Some(new_path)
        }
        (Some(current_path), Some(new_path)) if current_path != new_path => {
            diesel::update(points::dsl::points.find(point.id))
                .set(points::dsl::path.eq(new_path))
                .execute(connection)
                .expect("Error updating point");

            Some(new_path)
        }
        (Some(current_path), None) if fs::metadata(&current_path).is_err() => {
            diesel::update(points::dsl::points.find(point.id))
                .set(points::dsl::path.eq(new_path))
                .execute(connection)
                .expect("Error updating point");

            None
        }
        (Some(current_path), _) => Some(&current_path[..]),
        (None, _) => None,
    };

    if let Some(path) = path {
        for (tag_name, tag_content) in get_generic_tags_from_file(Path::new(path), magic_file) {
            tag_point(connection, point.id, tag_name, tag_content)
        }
    }

    if let Some(new_hash) = new_hash {
        if point.hash != new_hash {
            diesel::update(points::dsl::points.find(point.id))
                .set(points::dsl::hash.eq(new_hash))
                .execute(connection)
                .expect("Error updating point");
        }
    }
}

fn path_parts_to_tags(path_parts: &[&str]) -> TagEntries {
    let mut tags: TagEntries = Vec::new();

    for path_part in path_parts {
        let tag: TagEntry = match path_part
            .split('=')
            .map(|x| x.trim())
            .collect::<Vec<&str>>()[..]
        {
            [tag_name, tag_value, tag_sort_value] => (
                tag_name.to_string(),
                Some((
                    tag_value.to_string(),
                    Some(
                        tag_sort_value
                            .parse::<i64>()
                            .expect("Bad sort value encountered in store"),
                    ),
                )),
            ),
            [tag_name, tag_value] => (tag_name.to_string(), Some((tag_value.to_string(), None))),
            [tag_name] => (tag_name.to_string(), None),
            _ => panic!("Badly formatted path for dir in store path"),
        };

        tags.push(tag);
    }

    tags
}

fn store_path_to_name_and_tags(path: &Path) -> (String, TagEntries) {
    let name = path
        .file_name()
        .unwrap()
        .to_str()
        .expect("mfin unicode")
        .to_string();

    let tags = path_parts_to_tags(
        path.parent()
            .unwrap()
            .to_str()
            .unwrap()
            .split('/')
            .collect::<Vec<&str>>()
            .as_slice(),
    );

    (name, tags)
}

fn load_store(connection: &SqliteConnection, store_dir: &str, magic_file: &str) {
    let tags = match fs::read_to_string(format!("{}/@flat-info", store_dir)) {
        Ok(s) => path_parts_to_tags(s.split('/').collect::<Vec<&str>>().as_slice()),
        Err(_) => vec![],
    };

    for entry in walkdir::WalkDir::new(store_dir) {
        let entry = entry.unwrap();

        let path = entry.path();
        let rel_path = path
            .strip_prefix(store_dir)
            .expect("Path in store dir should be in store dir");

        if entry.path_is_symlink() {
            error!("Store path {:?} is a symlink, this is not supported", path);
            continue;
        }

        let mut split_dir = rel_path
            .iter()
            .map(|x| x.to_str().unwrap())
            .collect::<Vec<&str>>();

        if let ["@flat-info"] = split_dir.as_slice() {
            info!("Not importing @flat-info meta-file");
            continue;
        }

        split_dir.pop();

        if split_dir.contains(&"@dir") {
            continue;
        }

        let target: PathBuf = path.to_path_buf();

        let (name, mut new_tags) = if !entry.file_type().is_file() {
            if path.file_name().expect("Store entry dir path is invalid") != "@dir" {
                continue;
            }

            let rel_parent_path = rel_path.parent().expect("Store entry dir path is invalid");
            store_path_to_name_and_tags(rel_parent_path)
        } else {
            store_path_to_name_and_tags(rel_path)
        };

        let mut tags = tags.clone();
        tags.append(&mut new_tags);

        println!("{:?}: {:?} -> {:?}", name, tags, target);

        update_point_by_path(connection, name, target.to_str().unwrap(), magic_file, tags);
    }
}

fn main() {
    pretty_env_logger::init();
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config").required(false))
        .add_source(config::Environment::with_prefix("FFS"))
        .build()
        .expect("Error in config");

    let cfg = settings
        .try_deserialize::<FfsConfig>()
        .expect("Config not valid");

    let connection = SqliteConnection::establish(&cfg.db_url).expect("Error connecting to db");

    if let Some(store_dir) = cfg.store_dir {
        load_store(&connection, &store_dir, &cfg.magic_file);
    }

    for delegate_dir in cfg.delegate_dirs {
        load_store(&connection, &delegate_dir, &cfg.magic_file);
    }

    let mut args = env::args();

    match args.nth(1).unwrap_or_else(|| "".to_string()).as_str() {
        "mount" => {
            let mountpoint = match env::args_os().nth(2) {
                Some(mountpoint) => mountpoint,
                None => {
                    println!("where do i mount bitch");
                    return;
                }
            };

            let ffs = Ffs::new(connection);

            fuser::mount2(
                ffs,
                mountpoint,
                &[
                    // fuser::MountOption::AllowRoot,
                    // fuser::MountOption::RO,
                    // fuser::MountOption::AutoUnmount,
                ],
            )
            .unwrap();
        }
        "add" => {
            let path_str = match args.next() {
                Some(path) => path,
                None => {
                    println!("what file are u adding bitch");
                    return;
                }
            };

            let path = Path::new(&path_str);

            let full_path = fs::canonicalize(path).expect("error reading path");
            let full_path_str = full_path.to_str().unwrap();

            let mut tags = Vec::new();

            for t in args {
                let split = t.split('=').collect::<Vec<&str>>();

                let tag_name = match split.first() {
                    Some(x) => x.to_string(),
                    None => continue,
                };

                let tag_content = split.get(1).map(|x| (x.to_string(), x.parse::<i64>().ok()));

                tags.push((tag_name, tag_content));
            }

            let name = path
                .file_name()
                .expect("bad file path provided")
                .to_str()
                .unwrap()
                .to_string();

            update_point_by_path(&connection, name, full_path_str, &cfg.magic_file, tags);
        }
        "update-all" => {
            use schema::points;

            for point in points::dsl::points
                .load::<Point>(&connection)
                .expect("Error loading points")
            {
                update_point(&connection, &cfg.magic_file, None, None, &point);
            }
        }
        "remove" => {
            use schema::{joins, points};

            let id_str = match args.next() {
                Some(path) => path,
                None => {
                    println!("what point are u removing bitch");
                    return;
                }
            };

            let id = match id_str.parse::<i32>() {
                Ok(id) => id,
                Err(_) => {
                    println!("{:?} is not a valid ID", id_str);
                    return;
                }
            };

            diesel::delete(points::dsl::points.find(id))
                .execute(&connection)
                .expect("Error deleting point");

            diesel::delete(joins::dsl::joins.filter(joins::dsl::point_id.eq(id)))
                .execute(&connection)
                .expect("Error deleting point");

            println!("Deleted {:?}", id);
        }
        "tag" => {
            let id_str = match args.next() {
                Some(path) => path,
                None => {
                    println!("what point are u tagging bitch");
                    return;
                }
            };

            let id = match id_str.parse::<i32>() {
                Ok(id) => id,
                Err(_) => {
                    println!("{:?} is not a valid ID", id_str);
                    return;
                }
            };

            let tag_name = match args.next() {
                Some(path) => path,
                None => {
                    println!("what is the tag bitch");
                    return;
                }
            };

            let tag_content = args.next().map(|x| (x.to_string(), x.parse::<i64>().ok()));

            tag_point(&connection, id, tag_name, tag_content);
        }
        "untag" => {
            use schema::joins;

            let id_str = match args.next() {
                Some(path) => path,
                None => {
                    println!("what point are u tagging bitch");
                    return;
                }
            };

            let id = match id_str.parse::<i32>() {
                Ok(id) => id,
                Err(_) => {
                    println!("{:?} is not a valid ID", id_str);
                    return;
                }
            };

            let tag_name = match args.next() {
                Some(path) => path,
                None => {
                    println!("what is the tag query bitch");
                    return;
                }
            };

            let p = get_tags_by_parts(&connection, &[&tag_name]);

            let tag = match &p[0][..] {
                [x] => x,
                _ => {
                    println!("tag {:?} not found", tag_name);
                    return;
                }
            };

            diesel::delete(
                joins::dsl::joins
                    .filter(joins::dsl::point_id.eq(id))
                    .filter(joins::dsl::tag_id.eq(tag.id)),
            )
            .execute(&connection)
            .expect("Error deleting point");

            println!("Removed tag {:?} (id {:?}) from {:?}", tag_name, tag.id, id);
        }
        _ => {
            println!("CNF");
        }
    }
}
