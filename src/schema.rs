table! {
    joins (id) {
        id -> Integer,
        tag_id -> Integer,
        point_id -> Integer,
    }
}

table! {
    points (id) {
        id -> Integer,
        name -> Text,
        path -> Nullable<Text>,
        hash -> Text,
        dir -> Bool,
    }
}

table! {
    tags (id) {
        id -> Integer,
        name -> Text,
        value -> Nullable<Text>,
        sort_value -> Nullable<BigInt>,
    }
}

allow_tables_to_appear_in_same_query!(
    joins,
    points,
    tags,
);
