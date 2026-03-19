;; backlink_verified.lisp — example rule file showing graph-query DSL features.
;;
;; A note is marked "user.verified" when at least 3 *done* notes reference it
;; via backlinks. Demonstrates: backlinks, filter, length, >=, user-defined
;; metadata fields.
;;
;; To use: add this file to reconcile_rules in your zk-lsp config, alongside
;; the default checklist.lisp (or with disable_default = false).

(module
  (policy
    (cycle error)
    (unknown-status none))

  ;; Which metadata fields are materialized back to notes.
  (define (materialized_fields n)
    (list "checklist-status" "user.verified"))

  ;; Count how many notes that backlink to n are themselves done.
  (define (done_backlink_count n)
    (length (filter is_done_note (backlinks n))))

  ;; Predicate: is note n done?
  (define (is_done_note n)
    (done? (observe_meta n "checklist-status")))

  ;; Effective checkbox state: delegate to the raw observation.
  (define (effective_checked c)
    (observe_checked c))

  ;; Checkbox materialization: preserve the current source marker for `none`.
  (define (materialize_checked c)
    (if (done? (effective_checked c))
        checked
        (if (none? (effective_checked c))
            keep
            unchecked)))

  ;; Effective metadata: compute user.verified dynamically, pass through rest.
  (define (effective_meta n field)
    (if (eq? field "user.verified")
        (if (>= (done_backlink_count n) 3)
            "true"
            "false")
        (observe_meta n field))))
