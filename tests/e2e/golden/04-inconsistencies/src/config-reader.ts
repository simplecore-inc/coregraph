export function loadConfig(): Record<string, string | undefined> {
    return {
        database: process.env.database_url,
        redis: process.env.redis_url,
        log: process.env.log_level,
        port: process.env.app_port,
    };
}
