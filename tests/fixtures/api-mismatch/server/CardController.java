package com.example.card;

// Server exposes /api/v1/cards (plural).
@RestController
public class CardController {

    @GetMapping("/api/v1/cards")
    public List<Card> listCards() {
        return null;
    }

    @PostMapping("/api/v1/cards")
    public Card createCard(@RequestBody Card card) {
        return card;
    }
}
