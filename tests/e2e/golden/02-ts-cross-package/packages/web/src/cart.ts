import { Product, Order } from "../../shared/src/types";

export class Cart {
    private items: Array<{ product: Product; qty: number }> = [];

    add(product: Product, qty: number): void {
        this.items.push({ product, qty });
    }

    toOrders(): Order[] {
        return this.items.map((it, idx) => ({
            id: `order-${idx}`,
            productId: it.product.id,
            quantity: it.qty,
        }));
    }
}
