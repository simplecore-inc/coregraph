package com.example.config;

@Configuration
public class ServerConfig {

    @Value("${server.port}")
    private int port;

    @Value("${database.url}")
    private String dbUrl;
}
