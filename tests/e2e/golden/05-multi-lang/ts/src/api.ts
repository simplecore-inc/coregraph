export interface ApiRequest {
    path: string;
    method: "GET" | "POST";
}

export class ApiClient {
    async send(req: ApiRequest): Promise<Response> {
        return fetch(req.path, { method: req.method });
    }
}

export function healthCheck(): ApiRequest {
    return { path: "/health", method: "GET" };
}
