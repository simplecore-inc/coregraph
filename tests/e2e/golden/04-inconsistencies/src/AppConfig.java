import org.springframework.beans.factory.annotation.Value;

// Binds three of config.yaml's keys with Spring value injection, which the
// extractor records as config-file references (config_ref). That makes
// config.yaml "live application config", so its one remaining key that nothing
// binds (unused_key) is correctly reported as an unused config key.
//
// Note: config-reader.ts reads process.env.*, which is a RUNTIME env-var
// reference (env_ref), not a config-file binding, so it neither marks
// config.yaml live nor counts as referencing these keys. The config-file
// binding has to come from a value-injection reference, hence this Java file.
public class AppConfig {
    @Value("${database_url}")
    private String databaseUrl;

    @Value("${redis_url}")
    private String redisUrl;

    @Value("${log_level}")
    private String logLevel;
}
