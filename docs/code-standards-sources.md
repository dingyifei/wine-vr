# Sources for `docs/code-standards.md`

Numbers match the bracketed citations in the standard. Quotes are verbatim from the cited page or
chapter; a bracket like *[1 ch. 16]* points at a chapter of a book source.

1. John Ousterhout, *A Philosophy of Software Design* (2018; 2nd ed. 2021), ch. 13 "Comments Should
   Describe Things that Aren't Obvious from the Code", ch. 15 "Write the Comments First", ch. 16
   "Modifying Existing Code" ("Comments belong in the code, not the commit log"; "check the diffs").
2. Google C++ Style Guide, "Comments" — https://google.github.io/styleguide/cppguide.html#Comments
   (a definition comment may "explain why you chose to implement the function in the way you did
   rather than using a viable alternative").
3. Google Go Style Guide, "Clarity" and Decisions "Commentary" — https://google.github.io/styleguide/go/guide,
   https://google.github.io/styleguide/go/decisions#commentary
4. Go Doc Comments — https://go.dev/doc/comment ("Doc comments should not explain internal details
   such as the algorithm used in the current implementation.")
5. Google Python Style Guide §3.8.5 — https://google.github.io/styleguide/pyguide.html#385-block-and-inline-comments
   ("Never describe the code.")
6. Google Engineering Practices, "What to look for in a code review" —
   https://google.github.io/eng-practices/review/reviewer/looking-for.html (comments explain *why*;
   "look at comments that were there before this CL"; for pre-existing issues it allows "file a bug
   and add a TODO", which rule 2.3 deliberately overrides for false comments).
7. Robert C. Martin, *Clean Code* (2008), ch. 4 "Comments" (good: intent, clarification, warning of
   consequences, TODO, amplification; bad: redundant, misleading, journal, noise, position markers,
   commented-out code, bylines; "Inaccurate comments are far worse than no comments at all").
8. Linux kernel coding style, ch. 8 "Commenting" —
   https://www.kernel.org/doc/html/latest/process/coding-style.html#commenting ("tell WHAT your code
   does, not HOW").
9. The Rust Programming Language ch. 14.2 — https://doc.rust-lang.org/book/ch14-02-publishing-to-crates-io.html;
   The rustdoc book — https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html; Rust API
   Guidelines, "Documentation" (C-CRATE-DOC, C-EXAMPLE, C-FAILURE, C-LINK) —
   https://rust-lang.github.io/api-guidelines/documentation.html
10. Anders Abel, "Comments are not Version Control" (2012) —
    https://coding.abel.nu/2012/07/comments-are-not-version-control/
11. PEP 8, "Comments" — https://peps.python.org/pep-0008/#comments ("Comments that contradict the code
    are worse than no comments.")
12. Brian Kernighan and P. J. Plauger, *The Elements of Programming Style* (2nd ed. 1978) ("Make sure
    comments and code agree"; "Don't comment bad code — rewrite it").
13. Ellen Spertus, "Best practices for writing code comments", Stack Overflow blog (2021) —
    https://stackoverflow.blog/2021/12/23/best-practices-for-writing-code-comments/ ("A bad comment is
    worse than no comment at all.")
14. Alex Eagle, "Change-Detector Tests Considered Harmful", Google Testing Blog (2015) —
    https://testing.googleblog.com/2015/01/testing-on-toilet-change-detector-tests.html
15. Erik Kuefler, "Test Behaviors, Not Methods", Google Testing Blog (2014) —
    https://testing.googleblog.com/2014/04/testing-on-toilet-test-behaviors-not.html
16. Andrew Trenk, "Prefer Testing Public APIs Over Implementation-Detail Classes", Google Testing Blog
    (2015) — https://testing.googleblog.com/2015/01/testing-on-toilet-prefer-testing-public.html
17. Erik Kuefler, "Don't Put Logic in Tests", Google Testing Blog (2014) —
    https://testing.googleblog.com/2014/07/testing-on-toilet-dont-put-logic-in.html ("it's usually a
    good idea for any nontrivial test utility to have its own tests").
18. Ben Yu, "Keep Tests Focused", Google Testing Blog (2018) —
    https://testing.googleblog.com/2018/06/testing-on-toilet-keep-tests-focused.html
19. Dillon Bly, "Only Verify Relevant Method Arguments", Google Testing Blog (2018) —
    https://testing.googleblog.com/2018/06/testing-on-toilet-only-verify-relevant.html
20. Andrew Trenk, "Don't Overuse Mocks", Google Testing Blog (2013) —
    https://testing.googleblog.com/2013/05/testing-on-toilet-dont-overuse-mocks.html
21. Titus Winters, Tom Manshreck, Hyrum Wright, *Software Engineering at Google* (2020), ch. 11
    "Testing Overview" — https://abseil.io/resources/swe-book/html/ch11.html ("All tests should strive to
    be hermetic"; small tests may not touch disk or spawn processes) and ch. 12 "Unit Testing" —
    https://abseil.io/resources/swe-book/html/ch12.html ("write a test for each behavior"; "the bug fix
    should include that missing test case"; "test infrastructure must always have its own tests").
22. Kent Beck, "Test Desiderata" (2019) — https://testdesiderata.com/
23. Gerard Meszaros, *xUnit Test Patterns* (2007), "Test Smells" — http://xunitpatterns.com/Test%20Smells.html;
    "Eager Test" (a cause of Obscure Test) — http://xunitpatterns.com/Obscure%20Test.html
24. Ham Vocke, "The Practical Test Pyramid", martinfowler.com (2018) —
    https://martinfowler.com/articles/practical-test-pyramid.html ("duplicating tests throughout the
    different layers of the pyramid" is named a pitfall; "Push your tests as far down the test pyramid
    as you can").
25. Andrew Hunt and David Thomas, *The Pragmatic Programmer* (1999), tip 66 "Find Bugs Once".
26. Go wiki, "Table Driven Tests" — https://go.dev/wiki/TableDrivenTests; Dave Cheney, "Prefer table
    driven tests" (2019) — https://dave.cheney.net/2019/05/07/prefer-table-driven-tests
27. cargo-mutants book, "Welcome to cargo-mutants" — https://mutants.rs/; PIT, "What is mutation
    testing?" and "What's wrong with line coverage?" — https://pitest.org/; Goran Petrovic and Marko
    Ivankovic, "State of Mutation Testing at Google", ICSE-SEIP 2018 —
    https://research.google/pubs/state-of-mutation-testing-at-google/; Ezio Bartocci et al.,
    "Property-Based Mutation Testing", ICST 2023 — https://arxiv.org/abs/2301.13615 (defines a test
    case as *φ-redundant* w.r.t. a suite when the set of φ-killed mutants is unchanged by its inclusion;
    rule 3.4 uses the unqualified kill-set form).
