# Mission

Implémente entièrement le MVP décrit dans `SPEC.md`. Travaille de façon autonome et ne rends pas la main avant que tout le périmètre inclus soit réellement terminé, testé et documenté. Résous les erreurs rencontrées au lieu de livrer un état partiel.

Priorité absolue : livrer vite et économiser les tokens. Ne reformule pas la spec, ne produis pas de plan détaillé et ne demande pas confirmation pour les choix déjà tranchés. Inspecte l'existant, implémente directement par petites étapes, puis vérifie le résultat. Limite tes messages intermédiaires à une phrase utile.

## Économie de contexte obligatoire

- utilise `$caveman` pour réduire le coût et la verbosité ;
- préfixe toutes les commandes shell avec `rtk` et regroupe les lectures ou vérifications indépendantes ;
- utilise `rg`, `rg --files` et des lectures ciblées ; ne relis pas les fichiers inchangés et ne recopie jamais leur contenu dans tes réponses ;
- pour l'interface, applique `$antislop` en mode `during` avec la direction de `SPEC.md`, sans ajouter de décoration ou de texte marketing ;
- conserve uniquement dans le contexte les informations nécessaires à la prochaine modification ; résume les sorties longues et ne montre que les erreurs utiles ;
- n'utilise pas de sous-agent si le travail peut être réalisé directement à moindre coût.

Contraintes d'exécution :

- conserve et intègre le crate `proton-drive` existant dans le workspace ;
- utilise des crates Rust stables et connues, sans recherche web sauf blocage réel ;
- privilégie quelques modules clairs plutôt qu'une multiplication de crates ou d'abstractions ;
- définis le trait de stockage avant ses implémentations filesystem et Proton ;
- n'implémente aucune fonctionnalité marquée hors périmètre ;
- ne laisse aucun faux bouton, mock silencieux ou `TODO` sur le parcours principal ;
- en cas de détail non spécifié, choisis l'option la plus simple et continue.

Termine seulement lorsque `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` et `cargo test --workspace` réussissent. Ta réponse finale doit uniquement résumer ce qui fonctionne, les commandes exécutées et les limites réelles restantes.
