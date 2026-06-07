import { Order } from "../../shared/src/types";
import { ProductService } from "./productService";

export class OrderHandler {
    constructor(private products: ProductService) {}

    process(order: Order): number {
        return this.products.priceFor(order.productId, order);
    }
}
