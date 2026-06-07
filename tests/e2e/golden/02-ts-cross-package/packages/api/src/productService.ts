import { Product, Order, totalPrice } from "../../shared/src/types";

export class ProductService {
    private products: Product[] = [];

    add(p: Product): void {
        this.products.push(p);
    }

    get(id: string): Product | undefined {
        return this.products.find(x => x.id === id);
    }

    priceFor(productId: string, order: Order): number {
        const p = this.get(productId);
        if (!p) throw new Error("not found");
        return totalPrice(p, order);
    }
}
