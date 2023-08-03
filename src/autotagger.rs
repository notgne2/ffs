use std::collections::HashMap;

use crate::utils::*;
use magic::{Cookie, CookieFlags};
use std::fs;
use std::path::Path;

use id3::TagLike;

pub fn get_generic_tags_from_file(path: &Path, magic_file: &str) -> TagEntries {
    let cookie = Cookie::open(CookieFlags::default()).unwrap();
    cookie.load(&[magic_file]).expect("error loading magic");

    let magic_data = cookie.file(path).unwrap();

    // let mut tag_map: HashMap<String, (Option<String>, Option<i64>)> = HashMap::new();
    let mut tag_map: HashMap<String, Option<(String, Option<i64>)>> = HashMap::new();

    let mut is_image = false;

    for (i, magic_str) in magic_data.split(", ").enumerate() {
        if i == 0 {
            tag_map.insert(
                "type".to_string(),
                magic_str
                    .split_whitespace()
                    .next()
                    .map(|x| (x.to_lowercase(), None)),
            );

            if magic_str == "PNG image data" || magic_str == "JPEG image data" {
                is_image = true;
            }
        }

        if is_image {
            let resolution_split = magic_str.split('x').collect::<Vec<&str>>();

            if let [width, height] = &resolution_split[..] {
                let width = width.trim();
                let height = height.trim();

                let res = format!("{}x{}", width, height);

                tag_map.insert("resolution".to_string(), Some((res, None)));
                tag_map.insert(
                    "width".to_string(),
                    Some((width.to_string(), width.parse::<i64>().ok())),
                );
                tag_map.insert(
                    "height".to_string(),
                    Some((height.to_string(), height.parse::<i64>().ok())),
                );
            }

            let metadata_split = magic_str.split('=').collect::<Vec<&str>>();

            match &metadata_split[..] {
                ["manufacturer", manufacturer] => {
                    tag_map.insert(
                        "camera_manufacturer".to_string(),
                        Some(((*manufacturer).to_string(), None)),
                    );
                }
                ["software", software] => {
                    tag_map.insert(
                        "camera_software".to_string(),
                        Some(((*software).to_string(), None)),
                    );
                }
                ["model", model] => {
                    tag_map.insert(
                        "camera_model".to_string(),
                        Some(((*model).to_string(), None)),
                    );
                }
                _ => {}
            }
        }

        if magic_str == "directory" {
            if fs::metadata(path.to_str().expect("gotta b unicode bby").to_string() + "/.git")
                .is_ok()
            {
                tag_map.insert("code".to_string(), None);
                tag_map.insert("git".to_string(), None);
            }

            if fs::metadata(
                path.to_str().expect("gotta b unicode bby").to_string() + "/package.json",
            )
            .is_ok()
            {
                tag_map.insert("code".to_string(), None);
                tag_map.insert(
                    "language".to_string(),
                    Some(("javascript".to_string(), None)),
                );
                tag_map.insert("npm".to_string(), None);
            }

            if fs::metadata(path.to_str().expect("gotta b unicode bby").to_string() + "/Cargo.toml")
                .is_ok()
            {
                tag_map.insert("code".to_string(), None);
                tag_map.insert("language".to_string(), Some(("rust".to_string(), None)));
                tag_map.insert("cargo".to_string(), None);
            }

            if fs::metadata(path.to_str().expect("gotta b unicode bby").to_string() + "/elm.json")
                .is_ok()
            {
                tag_map.insert("code".to_string(), None);
                tag_map.insert("language".to_string(), Some(("elm".to_string(), None)));
                tag_map.insert("elm".to_string(), None);
            }
        }

        if magic_str == "ASCII text" {
            tag_map.insert("ascii".to_string(), None);
        }

        if magic_str == "C source" {
            tag_map.insert("code".to_string(), None);
            tag_map.insert("language".to_string(), Some(("c".to_string(), None)));
        }

        if magic_str == "C source" || magic_data == "ASCII text" {
            tag_map.insert("text".to_string(), None);

            match path.extension().map(|x| x.to_str().expect("unicode bitch")) {
                Some("rs") => {
                    tag_map.insert("code".to_string(), None);
                    tag_map.insert("language".to_string(), Some(("rust".to_string(), None)));
                }
                Some("js") => {
                    tag_map.insert("code".to_string(), None);
                    tag_map.insert(
                        "language".to_string(),
                        Some(("javascript".to_string(), None)),
                    );
                }
                Some("elm") => {
                    tag_map.insert("code".to_string(), None);
                    tag_map.insert("language".to_string(), Some(("elm".to_string(), None)));
                }
                Some("json") => {
                    tag_map.insert("language".to_string(), Some(("json".to_string(), None)));
                }
                Some("toml") => {
                    tag_map.insert("language".to_string(), Some(("toml".to_string(), None)));
                }
                Some("nix") => {
                    tag_map.insert("language".to_string(), Some(("nix".to_string(), None)));
                }
                Some("ini") => {
                    tag_map.insert("language".to_string(), Some(("ini".to_string(), None)));
                }
                _ => {}
            }
        }
        if magic_str == "Python script" {
            tag_map.insert("code".to_string(), None);
            tag_map.insert("language".to_string(), Some(("python".to_string(), None)));
        }

        if magic_str == "dynamically linked" {
            tag_map.insert("linker".to_string(), Some(("dynamic".to_string(), None)));
        }

        if magic_str.starts_with("ELF 64-bit") {
            tag_map.insert("elf".to_string(), None);
            tag_map.insert("arch".to_string(), Some(("x86_64".to_string(), None)));
        }

        if magic_str.starts_with("ELF 32-bit") {
            tag_map.insert("elf".to_string(), None);
            tag_map.insert("arch".to_string(), Some(("i686".to_string(), None)));
        }

        if magic_str == "Zip archive data" {
            tag_map.insert("archive".to_string(), None);
        }

        if magic_str.starts_with("MP4 ") {
            tag_map.insert("type".to_string(), Some(("mp4".to_string(), None)));
            tag_map.insert("video".to_string(), None);
        }

        if magic_str.contains(" ID3 ") {
            tag_map.insert(
                "type".to_string(),
                Some((
                    match path.extension().map(|x| x.to_str().expect("unicode bitch")) {
                        Some("mp3") => "mp3".to_string(),
                        _ => "audio".to_string(),
                    },
                    None,
                )),
            );

            let id3_tag = id3::Tag::read_from_path(path).expect("Error reading ID3 tag");

            if let Some(album) = id3_tag.album() {
                tag_map.insert("album".to_string(), Some((album.to_string(), None)));
            }

            if let Some(artist) = id3_tag.artist() {
                tag_map.insert("artist".to_string(), Some((artist.to_string(), None)));
            }

            if let Some(album_artist) = id3_tag.album_artist() {
                tag_map.insert(
                    "album_artist".to_string(),
                    Some((album_artist.to_string(), None)),
                );
            }

            // if let Some(title) = id3_tag.title() {
            //     tag_map.insert("title".to_string(), Some((title.to_string(), None)));
            // }

            if let Some(genre) = id3_tag.genre() {
                tag_map.insert("genre".to_string(), Some((genre.to_string(), None)));
            }

            if let Some(year) = id3_tag.year() {
                tag_map.insert(
                    "year".to_string(),
                    Some((year.to_string(), Some(year as i64))),
                );
            }

            let mut comments = id3_tag.comments();
            if let Some(comment) = comments.next() {
                tag_map.insert("comment".to_string(), Some((comment.text.clone(), None)));
            }
        }

        if magic_str == "WAVE audio" {
            tag_map.insert("type".to_string(), Some(("wav".to_string(), None)));
            tag_map.insert("audio".to_string(), None);
        }
    }

    tag_map.insert("magic".to_string(), Some((magic_data, None)));

    let mut tags: TagEntries = Vec::new();

    for (tag_name, tag_content) in tag_map.into_iter() {
        let tag_content_sanitised = tag_content.map(|(tag_value, maybe_tag_sort_value)| {
            (
                tag_value.trim_matches(char::from(0)).to_string(),
                maybe_tag_sort_value,
            )
        });

        tags.push((tag_name, tag_content_sanitised));
    }

    tags
}
