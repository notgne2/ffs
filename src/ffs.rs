use super::{get_points_by_parts, get_tags_for_point, get_tags_for_points, schema, Point, Tag};
use diesel::prelude::*;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyOpen, Request,
};
use libc::{ENOENT, ENOTDIR};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

trait FfsPathBuf {
    fn from_names(names: &[&str]) -> Self;
}

trait FfsPath {
    fn names(&self) -> PathNames<'_>;
}

#[derive(Clone)]
pub struct PathNames<'a> {
    inner: std::path::Components<'a>,
}

impl<'a> Iterator for PathNames<'a> {
    type Item = &'a str;

    #[inline]
    fn next(&mut self) -> Option<&'a str> {
        loop {
            match self.inner.next() {
                Some(Component::Normal(p)) => break Some(p.to_str().unwrap()),
                Some(_) => continue,
                None => break None,
            }
        }
    }
}

impl FfsPathBuf for PathBuf {
    fn from_names(names: &[&str]) -> Self {
        let mut path = PathBuf::new();

        for name in names {
            path.push(name);
        }

        path
    }
}

impl FfsPath for Path {
    fn names(&self) -> PathNames<'_> {
        PathNames {
            inner: self.components(),
        }
    }
}

pub struct Ffs {
    db: SqliteConnection,

    next_ino: AtomicU64,
    next_fh: AtomicU64,

    path_to_ino: HashMap<PathBuf, u64>,
    ino_to_path: HashMap<u64, PathBuf>,

    ino_to_point: HashMap<u64, Point>,

    fh_to_path: HashMap<u64, PathBuf>,

    extra_dirs: Vec<PathBuf>,

    dir_entries: HashMap<u64, Vec<(u64, FileType, String)>>,
}

const TTL: Duration = Duration::from_secs(1);

fn basic_directory(ino: u64) -> FileAttr {
    FileAttr {
        ino,
        size: 0,
        blocks: 0,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: FileType::Directory,
        perm: 0o755,
        nlink: 1,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
        blksize: 512,
    }
}

fn basic_link(ino: u64) -> FileAttr {
    FileAttr {
        ino,
        size: 0,
        blocks: 0,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: FileType::Symlink,
        perm: 0o755,
        nlink: 1,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
        blksize: 512,
    }
}

fn basic_file(ino: u64, size: u64, blocks: u64) -> FileAttr {
    FileAttr {
        ino,
        size,
        blocks,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: FileType::RegularFile,
        perm: 0o755,
        nlink: 1,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
        blksize: 512,
    }
}

enum ParsedPath<'a> {
    Flattened(Vec<&'a str>, Vec<&'a str>, Vec<&'a str>),
    Normal(Vec<&'a str>),
}

fn parse_path(path: &Path) -> ParsedPath {
    let path_names = path.names().collect::<Vec<&str>>();
    if let Some(flatten_pos) = path_names.iter().position(|&x| x == "@flatten") {
        let filter_path_names = path_names
            .iter()
            .take(flatten_pos)
            .copied()
            .collect::<Vec<&str>>();
        let flat_path_names = path_names
            .iter()
            .skip(flatten_pos + 1)
            .copied()
            .collect::<Vec<&str>>();

        let mut query_names: Vec<&str> = filter_path_names.clone();
        query_names.extend_from_slice(&flat_path_names);

        ParsedPath::Flattened(filter_path_names, flat_path_names, query_names)
    } else {
        ParsedPath::Normal(path_names)
    }
}

fn format_tag(tag: &Tag) -> String {
    match &tag.value {
        Some(v) => format!("{} = {}", tag.name, v),
        None => tag.name.to_string(),
    }
}

impl Ffs {
    pub fn new(connection: SqliteConnection) -> Ffs {
        Ffs {
            db: connection,

            // using ino 1 will cause problems lol
            // I guess the first dir to be added in readdir gets confused with the root dir
            // i.e. it thinks everything is in /@flatten
            next_ino: AtomicU64::new(2),
            next_fh: AtomicU64::new(1),

            path_to_ino: HashMap::new(),
            ino_to_path: HashMap::new(),

            ino_to_point: HashMap::new(),

            fh_to_path: HashMap::new(),

            extra_dirs: Vec::new(),

            dir_entries: HashMap::new(),
        }
    }

    pub fn lookup_point_by_name(&mut self, path: &Path) -> Option<Point> {
        if let Some(last_part) = path.file_name() {
            use schema::points;

            if let Some(Ok(possible_id)) = last_part
                .to_str()
                .unwrap()
                .split('.')
                .last()
                .map(|x| x.parse::<i32>())
            {
                if let Ok(point_for_id) = points::dsl::points
                    .find(possible_id)
                    .first::<Point>(&self.db)
                {
                    return Some(point_for_id);
                }
            }
        }

        None
    }

    pub fn new_fh(&mut self, path: &Path) -> u64 {
        let ino = self.next_fh.fetch_add(1, Ordering::SeqCst);
        self.fh_to_path.insert(ino, path.to_owned());
        ino
    }

    pub fn read_fh(&self, fh: u64, maybe_ino: Option<u64>) -> Option<&Path> {
        match (self.fh_to_path.get(&fh).map(|x| x.as_path()), maybe_ino) {
            (Some(x), _) => Some(x),
            (None, Some(ino)) => self.read_ino(ino),
            (None, None) => None,
        }
    }

    pub fn new_ino(&mut self, path: &Path) -> u64 {
        if let Some(x) = self.path_to_ino.get(path) {
            *x
        } else {
            let ino = self.next_ino.fetch_add(1, Ordering::SeqCst);
            self.path_to_ino.insert(path.to_owned(), ino);
            self.ino_to_path.insert(ino, path.to_owned());
            ino
        }
    }

    pub fn read_ino(&self, ino: u64) -> Option<&Path> {
        self.ino_to_path.get(&ino).map(|x| x.as_path())
    }

    fn internal_lookup(
        &mut self,
        path: &Path,
        maybe_parent_ino: Option<u64>,
    ) -> Result<FileAttr, ()> {
        let name = path.file_name().unwrap_or(OsStr::new("")).to_str().unwrap();

        match parse_path(&path) {
            ParsedPath::Flattened(filter_path_names, flat_path_names, query_names) => {
                let query_path = PathBuf::from_names(&query_names);

                if let Some(point) = self.lookup_point_by_name(&query_path) {
                    let point_full_name = format!("{}.{}", point.name, point.id);
                    let ino = self.new_ino(&path.join(&point_full_name));
                    self.ino_to_point.insert(ino, point.clone());

                    if point.dir {
                        return Ok(basic_directory(ino));
                    } else {
                        return Ok(basic_link(ino));
                    }
                } else if flat_path_names.is_empty() {
                    // This is for the @flatten dir itself
                    return Ok(basic_directory(self.new_ino(&path)));
                } else if flat_path_names == ["@flat-info"] {
                    // Return info for the @flat-info meta-file found in @flatten dirs
                    return Ok(basic_file(
                        self.new_ino(&path),
                        // The file returns the filter path string, so we need to tell what size it will be
                        usize::saturating_sub(
                            filter_path_names.iter().fold(0, |a, p| a + p.len() + 1),
                            1,
                        ) as u64,
                        // idk lol
                        1,
                    ));
                } else {
                    let (flat_parent_path_names, flat_name) = match flat_path_names.split_last() {
                        Some((flat_name, flat_parent_path_names)) => {
                            (flat_parent_path_names, *flat_name)
                        }
                        None => (&[] as &[&str], ""),
                    };

                    let mut parent_query_names: Vec<&str> = filter_path_names.clone();
                    parent_query_names.extend_from_slice(flat_parent_path_names);

                    // If the last part of the path is @dir, then the parent should be a flat dir point, so resolve us as whatever point that is
                    if flat_name == "@dir" {
                        // Parent ino won't always be surprised, sometimes we may need to get it by looking up the ino of the parent path
                        let parent_ino = match maybe_parent_ino {
                            Some(x) => x,
                            None => match self.path_to_ino.get(path.parent().unwrap()) {
                                Some(x) => *x,
                                None => return Err(()),
                            },
                        };

                        if let Some(point) = self.ino_to_point.get(&parent_ino) {
                            let point = point.clone();
                            let point_full_name = format!("{}.{}", point.name, point.id);
                            let ino = self.new_ino(&path.join(&point_full_name));
                            self.ino_to_point.insert(ino, point.clone());

                            return Ok(basic_link(ino));
                        }
                    }

                    // Iterate over tags for points that match our parent's query
                    // We do this so we can find the first tag that applies to every point within ourselves
                    // Then if we see ourselves, we are a valid flat tag dir
                    for point in get_points_by_parts(&self.db, &parent_query_names) {
                        let tags = get_tags_for_point(&self.db, &point);

                        let mut full_tags = Vec::new();

                        for tag in tags {
                            let full_tag_name = format_tag(&tag);
                            if !parent_query_names.contains(&full_tag_name.as_str()) {
                                full_tags.push(full_tag_name);
                            }
                        }

                        full_tags.sort();

                        if let Some(first_tag_of_point) = full_tags.first() {
                            if first_tag_of_point == flat_name {
                                return Ok(basic_directory(self.new_ino(&path)));
                            }
                        }
                    }
                }
            }
            ParsedPath::Normal(path_names) => {
                // For the @flatten directory itself
                if name == "@flatten" {
                    return Ok(basic_directory(self.new_ino(&path)));
                }

                // If directory was created, show it
                if self.extra_dirs.contains(&path.to_path_buf()) {
                    return Ok(basic_directory(self.new_ino(&path)));
                }

                // If this is the root dir itself
                if path_names.len() == 0 {
                    return Ok(basic_directory(self.new_ino(&path)));
                }

                match self.lookup_point_by_name(&path) {
                    Some(_) => {
                        return Ok(basic_link(self.new_ino(&path)));
                    }
                    None => {
                        let points = get_points_by_parts(&self.db, &path_names);

                        let tags = get_tags_for_points(&self.db, &points);

                        if tags
                            .iter()
                            .map(|x| match &x.value {
                                Some(v) => format!("{} = {}", x.name, v),
                                None => x.name.to_string(),
                            })
                            .any(|x| x == *name)
                        {
                            return Ok(basic_directory(self.new_ino(&path)));
                        }
                    }
                }
            }
        }

        return Err(());
    }
}

impl Filesystem for Ffs {
    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        let path = match ino {
            1 => Path::new(""),
            _ => match self.read_ino(ino) {
                Some(p) => p,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            },
        };
        let path = path.to_path_buf();

        let file_attr = match self.internal_lookup(&path, None) {
            Ok(x) => x,
            Err(_) => {
                reply.error(ENOENT);
                return;
            }
        };

        reply.attr(&TTL, &file_attr);
    }

    fn lookup(&mut self, _req: &Request, parent_ino: u64, name_os_str: &OsStr, reply: ReplyEntry) {
        let maybe_parent_path = self.read_ino(parent_ino);
        let path = match maybe_parent_path {
            None => PathBuf::from(name_os_str),
            Some(x) => Path::new(x).join(name_os_str),
        };

        let file_attr = match self.internal_lookup(&path, Some(parent_ino)) {
            Ok(x) => x,
            Err(_) => {
                reply.error(ENOENT);
                return;
            }
        };

        reply.entry(&TTL, &file_attr, 0);
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if let Some(entries) = self.dir_entries.get(&ino) {
            for (i, entry) in entries.iter().enumerate().skip(offset as usize) {
                reply.add(entry.0, (i + 1) as i64, entry.1, entry.2.clone());
            }

            // Cache should only be used once (for staggered reads), delete once it's done reading
            if offset == (entries.len() as i64) {
                self.dir_entries.remove(&ino);
            }
        } else {
            let mut entries: Vec<(u64, FileType, String)> = vec![
                (1, FileType::Directory, ".".to_string()),
                (1, FileType::Directory, "..".to_string()),
            ];

            let path = self.read_ino(ino).unwrap_or_else(|| Path::new(""));
            let path = &path.to_path_buf();

            match parse_path(&path) {
                ParsedPath::Flattened(_, flat_path_names, query_names) => {
                    // Show @flat-info file at top of @flatten dir
                    if flat_path_names.is_empty() {
                        entries.push((
                            self.new_ino(&path.join("@flat-info")),
                            FileType::RegularFile,
                            "@flat-info".to_string(),
                        ))
                    }

                    let query_path = PathBuf::from_names(&query_names);

                    if let Some(point) = self.lookup_point_by_name(&query_path) {
                        if !point.dir {
                            reply.error(ENOTDIR);
                            return;
                        }

                        entries.push((
                            self.new_ino(&path.join("@dir")),
                            FileType::RegularFile,
                            "@dir".to_string(),
                        ));
                    } else {
                        let points = get_points_by_parts(
                            &self.db,
                            &query_path.names().collect::<Vec<&str>>(),
                        );

                        let mut added_tags: Vec<String> = Vec::new();

                        for point in points {
                            let tags = get_tags_for_point(&self.db, &point);

                            let mut full_tags = Vec::new();

                            for tag in tags {
                                let full_tag_name = format_tag(&tag);
                                if !query_path
                                    .iter()
                                    .any(|x| x.to_str().unwrap() == full_tag_name.as_str())
                                {
                                    full_tags.push(full_tag_name);
                                }
                            }

                            full_tags.sort();

                            if let Some(first_tag) = full_tags.first() {
                                if added_tags.contains(&first_tag) {
                                    continue;
                                }

                                added_tags.push(first_tag.clone());
                                entries.push((
                                    self.new_ino(&path.join(&first_tag)),
                                    FileType::Directory,
                                    first_tag.clone(),
                                ));
                            } else {
                                if point.path.is_none() {
                                    continue;
                                }

                                let point_full_name = format!("{}.{}", point.name, point.id);
                                let ino = self.new_ino(&path.join(&point_full_name));
                                self.ino_to_point.insert(ino, point.clone());

                                entries.push((
                                    ino,
                                    if point.dir {
                                        FileType::Directory
                                    } else {
                                        FileType::Symlink
                                    },
                                    point_full_name,
                                ));
                            }
                        }
                    }
                }
                ParsedPath::Normal(path_names) => {
                    entries.push((
                        self.new_ino(&path.join("@flatten")),
                        FileType::Directory,
                        "@flatten".to_string(),
                    ));

                    for extra_dir in self.extra_dirs.clone() {
                        let extra_dir_names = extra_dir.names().collect::<Vec<&str>>();

                        // Show this extra directory if it's a child of ourselves
                        if let Some((extra_dir_name, extra_dir_parent_path)) =
                            extra_dir_names.split_last()
                        {
                            if extra_dir_parent_path == path_names {
                                entries.push((
                                    self.new_ino(&extra_dir),
                                    FileType::Directory,
                                    extra_dir_name.to_string(),
                                ));
                            }
                        }
                    }

                    let points = get_points_by_parts(&self.db, &path_names);
                    let tags = get_tags_for_points(&self.db, &points);

                    for point in points.iter().filter(|x| !x.path.is_none()) {
                        let point_full_name = format!("{}.{}", point.name, point.id);
                        let ino = self.new_ino(&path.join(&point_full_name));
                        self.ino_to_point.insert(ino, point.clone());

                        entries.push((
                            self.new_ino(&path.join(&point_full_name)),
                            FileType::Symlink,
                            point_full_name,
                        ));
                    }

                    for tag in tags {
                        let full_tag_name = format_tag(&tag);

                        // Don't add tags that are already in the previous path
                        if path_names.iter().any(|x| x == &full_tag_name.as_str()) {
                            continue;
                        }

                        entries.push((
                            self.new_ino(&path.join(&tag.name)),
                            FileType::Directory,
                            full_tag_name,
                        ));
                    }
                }
            }

            for (i, entry) in entries.iter().enumerate().skip(offset as usize) {
                reply.add(entry.0, (i + 1) as i64, entry.1, entry.2.clone());
            }

            if offset == 0 {
                self.dir_entries.insert(ino, entries);
            }
        };

        reply.ok();
    }

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        match self.ino_to_point.get(&ino) {
            Some(Point { path: Some(p), .. }) => reply.data(p.as_bytes()),
            _ => reply.error(ENOENT),
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request,
        parent: u64,
        name_os_str: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let path = match self.read_ino(parent) {
            None => PathBuf::from(name_os_str),
            Some(x) => Path::new(x).join(name_os_str),
        };

        reply.entry(&TTL, &basic_directory(self.new_ino(&path)), 0);
        self.extra_dirs.push(path);
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        let Some(path) = self.read_ino(ino) else {
            reply.error(ENOENT);
            return;
        };
        let path = &path.to_path_buf();
        reply.opened(self.new_fh(path), flags as u32);
    }

    fn flush(&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        _offset: i64,
        _size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let Some(path) = self.read_fh(fh, Some(ino)) else {
            reply.error(ENOENT);
            return;
        };

        let path_names = path.names().collect::<Vec<&str>>();

        if let [filter_str @ .., "@flatten", "@flat-info"] = path_names.as_slice() {
            reply.data(filter_str.join("/").as_bytes());
        } else {
            reply.error(ENOENT);
        }
    }
}
