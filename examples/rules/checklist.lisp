; Default reconcile module — reproduces existing workspace semantics.
;
; Semantic notes:
; - local_checkboxes(n) returns all checklist items in source order.
; - children(c) returns direct child checklist items in source order.
; - observe_meta n "checklist-status" is the fallback when no checklist items are present.
; - Archived notes are always considered Done (expressed in DSL via relation check).
; - Local non-leaf parents ignore their own checkbox marker; their children decide them.

(module
  (policy
    (cycle error)
    (unknown-status todo)
    (unknown-checked false))

  (define (effective_checked c)
    (and (self_truth c)
         (children_truth c)))

  (define (self_truth c)
    (if (empty? (targets c))
        (if (empty? (children c))
            (observe_checked c)
            true)
        (all_done (map target_status (targets c)))))

  (define (children_truth c)
    (if (empty? (children c))
        true
        (eq? (aggregate_status (map effective_checked (children c))) done)))

  (define (target_status n)
    (effective_meta n "checklist-status"))

  (define (effective_meta n field)
    (if (eq? field "checklist-status")
        (if (eq? (observe_meta n "relation") "archived")
            done
            (if (empty? (local_checkboxes n))
                (observe_meta n "checklist-status")
                (aggregate_status (map effective_checked (local_checkboxes n)))))
        (observe_meta n field))))
