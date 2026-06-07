package com.example.config;

// Server reads config keys. Note: "database.pool.maxsize" (no hyphen) is a
// typo that should be flagged — the YAML file defines "database.pool.max-size".
// Additionally "feature.caching.enabled" doesn't exist in application.yml.
@Configuration
public class AppConfig {

    @Value("${server.port}")
    private int serverPort;

    @Value("${database.url}")
    private String dbUrl;

    @Value("${database.pool.maxsize}")
    private int dbPoolMaxSize;

    @Value("${feature.caching.enabled}")
    private boolean cachingEnabled;
}
