# Perturb-seq: unreviewed paper excerpts

Joseph M. Replogle et al.; Cell / Elsevier; author full text archived by PMC

Original: https://www.ebi.ac.uk/europepmc/webservices/rest/PMC9380471/fullTextXML

License: https://creativecommons.org/licenses/by/4.0/

Changes: selected paragraphs; markup removed and whitespace collapsed. Inline reference labels retained. No scientific annotations approved.

## ex-perturb-seq-2022-01 (P6)

Mapping the relationship between genetic changes and their phenotypic consequence is critical to understanding gene and cellular function. This mapping is traditionally carried out in one of two ways: a phenotype-centric, ‘‘forward genetic’’ approach that reveals the genetic changes that drive a phenotype of interest or a gene-centric, ‘‘reverse genetic’’ approach that catalogs the diverse phenotypes caused by a defined genetic change.

## ex-perturb-seq-2022-02 (P7)

Recent technological developments have advanced both forward and reverse genetic efforts (Camp et al., 2019). CRISPR tools now enable the deletion, mutation, repression, or activation of genes with ease (Doench, 2018). In forward genetic screens, CRISPR-Cas systems can be used to generate pools of cells with diverse genetic perturbations, which can then be subjected to selection followed by sequencing to assign phenotypes to genetic perturbations. Forward genetic screens are powerful tools for the identification of cancer dependencies, essential cellular machinery, differentiation factors, and suppressors of genetic diseases (Kramer et al., 2018; Tsherniak et al., 2017; Wang et al., 2021, 2015). In parallel, dramatic improvements in molecular phenotyping now allow for single-cell readouts of epigenetic, transcriptomic, proteomic, and imaging information (Stuart and Satija, 2019). Applied to reverse genetics, single-cell profiling can refine the understanding of how select genetic perturbations affect cell types and cell states.

## ex-perturb-seq-2022-03 (P8)

However, both phenotype-centric and gene-centric approaches suffer conceptual and technical limitations. Pooled forward genetic screens typically use low-dimensional phenotypes such as growth or marker expression for selection. The use of simple phenotypes can conflate genes acting via different mechanisms, requiring extensive follow-up studies to disentangle genetic pathways (Przybyla and Gilbert, 2021). Additionally, in forward genetics, serendipitous discovery is constrained by the prerequisite of selecting phenotypes prior to screening. On the other hand, reverse genetic approaches enable the study of multidimensional and complex phenotypes but have typically been restricted in scale to rationally chosen targets, limiting systematic comparisons.

## ex-perturb-seq-2022-04 (P9)

As a solution to these problems, single-cell CRISPR screens simultaneously read out the genetic perturbation and high-dimensional phenotype of individual cells in a pooled format, thus combining the throughput of forward genetics with the rich phenotypes of reverse genetics. Although these approaches initially focused on transcriptomic phenotypes (e.g., Perturb-seq, CROP-seq) (Adamson et al., 2016; Datlinger et al., 2017; Dixit et al., 2016; Jaitin et al., 2016; Replogle et al., 2020), technical advances have enabled their application to epigenetic (Rubin et al., 2019), imaging (Feldman et al., 2019), or multimodal phenotypes (Mimitou et al., 2019). From these rich data, it is possible to identify genetic perturbations that cause a specific behavior as well as to catalog the spectrum of phenotypes associated with each genetic perturbation. Despite the promise of single-cell CRISPR screens, their use has generally been limited to studying at most a few hundred genetic perturbations chosen to address predefined biological questions.

## ex-perturb-seq-2022-05 (P10)

We reasoned that there would be unique value to genome-scale single-cell CRISPR screens. For example, although the number of perturbations scales linearly with experimental cost, the number of pairwise comparisons in a screen—and thus its utility for unsupervised classification of gene function—scales quadratically. Similarly, in large-scale screens, the diversity of perturbations allows exploration of the range of cell states that can be revealed by rich phenotypes. Additionally, as many human genes are well characterized, these genes serve as natural controls to anchor interpretation of comprehensive datasets. Finally, genome-scale experiments could help address fundamental questions, such as what fraction of genetic changes elicit transcriptional phenotypes and how transcriptional responses differ between cell types, with implications for understanding organizing principles of cells.

## ex-perturb-seq-2022-06 (P11)

Here, we report results from the first genome-scale Perturb-seq screens. We use a compact, multiplexed CRISPR interference (CRISPRi) library to assay thousands of loss-of-function genetic perturbations with single-cell RNA sequencing (scRNA-seq) in chronic myeloid leukemia (CML) (K562) and retinal pigment epithelial (RPE1) cell lines. Leveraging the scale and diversity of these perturbations, we show that Perturb-seq can be used to study numerous complex cellular phenotypes—from RNA splicing to differentiation to chromosomal instability (CIN)—and discover gene functions. We then invert our analysis to focus on regulatory networks and uncover unanticipated stress-specific regulation of the mitochondrial genome. In sum, we use Perturb-seq to reveal a multidimensional portrait of cellular behavior, gene function, and regulatory networks that advances the goal of creating comprehensive genotype-phenotype maps.

## ex-perturb-seq-2022-07 (P12)

Perturb-seq uses scRNA-seq to concurrently read out the CRISPR single-guide RNAs (sgRNAs) (i.e., genetic perturbation) and transcriptome (i.e., high-dimensional phenotype) of single cells in a pooled format (Figure 1A). We sought to exploit and understand the rich information content of transcriptomic phenotypes by studying a comprehensive set of genetic perturbations in a given cell type. To enable genome-scale Perturb-seq, we considered key parameters that would increase scalability and data quality, such as the genetic perturbation modality and sgRNA library.

## ex-perturb-seq-2022-08 (P13)

Perturb-seq is compatible with a range of CRISPR-based perturbations. We elected to use CRISPRi for several reasons: (1) Compared with gain-of-function perturbations, a higher proportion of loss-of-function perturbations yield phenotypes in growth and chemical-genetic screens, especially for members of protein complexes (Gilbert et al., 2014; Horlbeck et al., 2016). (2) CRISPRi allows direct measurement of the efficacy of genetic perturbation—knockdown—by scRNA-seq. Exploiting this feature allowed us to target each gene in our library with a single element and empirically exclude unperturbed genes from downstream analysis. (3) CRISPRi tends to yield more homogeneous perturbation than CRISPR knockout, which can generate active in-frame indels (Smits et al., 2019). The relative homogeneity of CRISPRi reduces selection for unperturbed cells, especially when studying essential genes. (4) Unlike CRISPR knockout, CRISPRi does not lead to activation of the DNA damage response which can alter transcriptional signatures (Haapaniemi et al., 2018).

## ex-perturb-seq-2022-09 (P14)

We first optimized our CRISPRi sgRNA libraries for scalability. To maximize CRISPRi efficacy, we used multiplexed CRISPRi libraries in which each element contains two distinct sgRNAs targeting the same gene (Table S1; Replogle et al., 2020). To avoid low representation of sgRNAs targeting essential genes, we performed growth screens and, during oligonucleotide library synthesis, overrepresented constructs that caused strong growth defects (Figures S1A–S1D).

## ex-perturb-seq-2022-10 (P15)

Next, we devised a three-pronged Perturb-seq screening approach encompassing multiple time points and cell types (Figure 1A). As a principal cell line, we studied CML K562 cells engineered to express the CRISPRi effector dCas9-KRAB (Gilbert et al., 2014). In this cell line, we performed two Perturb-seq screens: one targeting all expressed genes sampled 8 days after lentiviral transduction (n = 9,866 genes) and another targeting common essential genes sampled 6 days after transduction (n = 2,057 genes). As a secondary cell line, we used RPE1 cells engineered to express dCas9 fused to a ZIM3-derived KRAB domain, which was recently shown to improve CRISPRi transcriptional repression (Alerasool et al., 2020), sampled 7 days after transduction. In contrast to K562 cells, RPE1 cells are a non-cancerous, hTERT-immortalized, near-euploid, adherent, and p53-positive cell line.
