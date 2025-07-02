//! Builder for [`DbEnv`].

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_default() {
        let builder = DbEnvBuilder::default();
        assert!(matches!(builder.kind, DbEnvKind::RW));
        assert!(!builder.ephemeral);
        assert_eq!(builder.version.inner(), CURRENT_DB_VERSION.inner());
    }

    #[test]
    fn builder_ephemeral() {
        let db = DbEnvBuilder::new(DbEnvKind::RW)
            .ephemeral()
            .build_ephemeral()
            .expect("Failed to create ephemeral database");

        assert!(db.path().exists());
    }

    #[test]
    fn builder_custom_settings() {
        let builder = DbEnvBuilder::new(DbEnvKind::RO)
            .max_readers(1000)
            .max_dbs(50)
            .with_version(Version::new(42));

        assert!(matches!(builder.kind, DbEnvKind::RO));
        assert_eq!(builder.max_readers, Some(1000));
        assert_eq!(builder.max_dbs, Some(50));
        assert_eq!(builder.version.inner(), 42);
    }
}
