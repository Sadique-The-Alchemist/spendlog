diesel::table! {
    ledgers (id) {
        id -> Int4,
        #[max_length = 10]
        code -> Varchar,
        #[max_length = 100]
        name -> Varchar,
        description -> Nullable<Text>,
        #[max_length = 10]
        sort -> Varchar,
        #[max_length = 20]
        kind -> Varchar,
        created_at -> Nullable<Timestamp>,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    proceedings (id) {
        id -> Int4,
        cr_from -> Int4,
        db_to -> Int4,
        amount -> Float8,
        narration -> Text,
        created_at -> Nullable<Timestamp>,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(ledgers, proceedings,);
