//! Track 2 (History Sync): ZDB4.0 RSA/AES archive unpacker, LZMA/XZ, read-only SQLite3, BinNet TLV importer.

pub mod lzma;
pub mod sqliteread;
pub mod syncimport;
pub mod zdb;

pub use lzma::{
    decode_lzma_alone, decode_lzma_alone_with_hint, decode_lzma_auto, decode_lzma_auto_with_hint,
    decode_xz, decode_xz_with_hint, LzmaError, DEFAULT_MAX_OUTPUT, XZ_MAGIC,
};
pub use sqliteread::{
    is_sqlite, parse_create_table, parse_record, read_table, read_table_rows, SqlRow, SqlTable,
    SqlTableRows, SqlValue, SqliteError,
};
pub use syncimport::{
    convert_conversation, describe_senders, is_outgoing, is_placeholder_caption, parse_bin_net,
    read_conversation, to_millis, BackupConversation, BackupIdMap, BackupRow, BinNet,
    BinNetAttachment, BinNetMention, BinNetQuote, ConvertResult, ConvertedMessage,
};
pub use zdb::{
    backup_key_text, conversation_files, conversation_files_owned, decrypt_backup_key,
    decrypt_chunks_in_place, decrypt_chunks_raw, decrypt_zdb_chunks, decrypt_zdb_chunks_owned,
    derive_backup_aes_key, encrypt_backup_key_pkcs1, encrypt_chunks_raw, generate_sync_keypair,
    is_conversation_db, pack_zdb_backup, unpack_backup, unpack_backup_owned, unpack_zdb,
    unpack_zdb_owned, BackupError, ConversationFile, SyncKeyPair, ZdbEntry, MAGIC, MAX_FILES,
};
